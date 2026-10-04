-- lavalamp
-- a Portamax norns script
--
-- warm wax blobs drift up and down
-- a glass lamp. each blob hums a
-- soft note as it turns at the top
-- or bottom; when two blobs merge
-- they play a chord and become one.
--
-- E2 heat   E3 tone
-- K2 split a blob   K3 lamp off / on
-- pads: drop in a new blob
-- (params: root, chord quality, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local QUAL = { "major 7", "minor 7", "sus2", "sus4", "major", "minor" }
local blobs = {}
local scale = {}
local lit = true
local flash = 0

local function build()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), "Dorian", 12)
end

local function new_blob(x, y, r)
  return { x = x or 50 + math.random() * 28, y = y or 20 + math.random() * 30,
    r = r or 3 + math.random() * 3, vy = (math.random() - 0.5) * 0.6, deg = math.random(#scale) }
end

local function note(deg, amp)
  engine.amp(amp or 0.2)
  engine.hz(MusicUtil.note_num_to_freq(scale[util.clamp(deg, 1, #scale)]))
end

local function chord(deg)
  local c = MusicUtil.generate_chord(scale[util.clamp(deg, 1, #scale)] - 12, QUAL[params:get("qual")], 0)
  engine.amp(0.16)
  for i, n in ipairs(c) do
    engine.pan((i - 2) * 0.4)
    engine.hz(MusicUtil.note_num_to_freq(n))
  end
  engine.pan(0)
  flash = 8
end

function init()
  params:add_separator("LAVALAMP")
  params:add_number("root", "root", 36, 60, 48, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build)
  params:add_option("qual", "chord quality", QUAL, 1)
  params:add_control("heat", "heat", controlspec.new(0.2, 3, 'lin', 0, 1.2, ''))
  params:add_control("tone", "tone", controlspec.new(300, 5000, 'exp', 0, 1200, 'hz'))
  params:set_action("tone", function(x) engine.cutoff(x) end)
  params:add_control("release", "release", controlspec.new(0.5, 5, 'lin', 0, 2.5, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:default()
  engine.pw(0.5)
  math.randomseed(os.time())
  build()
  for i = 1, 5 do blobs[i] = new_blob() end
  note(blobs[1].deg)
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      local b = new_blob(nil, 52, 4)
      b.deg = (msg.note - 60) % #scale + 1
      b.vy = -0.8
      table.insert(blobs, b)
      note(b.deg, 0.25)
    end
  end
  clock.run(function()
    while true do
      clock.sleep(1 / 25)
      if lit then step() end
      flash = math.max(0, flash - 1)
      redraw()
    end
  end)
end

function step()
  local heat = params:get("heat")
  for _, b in ipairs(blobs) do
    local was = b.vy
    -- hot wax rises near the bottom, cools and sinks near the top
    b.vy = b.vy + (b.y > 40 and -0.02 or (b.y < 20 and 0.02 or 0)) * heat
    b.vy = util.clamp(b.vy, -0.9, 0.9)
    b.y = b.y + b.vy * heat * 0.6
    b.x = util.clamp(b.x + math.sin(b.y / 7) * 0.15, 48, 80)
    if b.y < 14 or b.y > 54 then b.vy = -b.vy * 0.5 b.y = util.clamp(b.y, 14, 54) end
    if (was < 0) ~= (b.vy < 0) and math.random() < 0.5 then note(b.deg, 0.14) end
  end
  for i = #blobs, 1, -1 do
    for j = i - 1, 1, -1 do
      local a, b = blobs[i], blobs[j]
      if a and b and math.abs(a.x - b.x) < (a.r + b.r) * 0.7 and math.abs(a.y - b.y) < (a.r + b.r) * 0.7 then
        b.r = math.min(9, math.sqrt(a.r * a.r + b.r * b.r))
        b.deg = (a.deg + b.deg) // 2
        chord(b.deg)
        table.remove(blobs, i)
        break
      end
    end
  end
  -- big blobs eventually split again
  if #blobs < 3 or (math.random() < 0.004 * heat) then split() end
end

function split()
  table.sort(blobs, function(a, b) return a.r > b.r end)
  local big = blobs[1]
  if big and big.r > 4 then
    big.r = big.r * 0.7
    local c = new_blob(big.x, big.y + big.r + 4, big.r)
    c.deg = util.clamp(big.deg + math.random(-3, 3), 1, #scale)
    c.vy = 0.5
    table.insert(blobs, c)
  else
    table.insert(blobs, new_blob())
  end
end

function enc(n, d)
  if n == 2 then params:delta("heat", d)
  elseif n == 3 then params:delta("tone", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then split() note(blobs[#blobs].deg, 0.22)
  elseif n == 3 then lit = not lit end
  redraw()
end

function redraw()
  screen.clear()
  screen.level(2)
  screen.move(48, 12) screen.line(42, 56) screen.stroke()
  screen.move(80, 12) screen.line(86, 56) screen.stroke()
  screen.level(lit and 4 or 1)
  screen.rect(40, 56, 48, 4) screen.fill()
  for _, b in ipairs(blobs) do
    screen.level(lit and (flash > 0 and 15 or 10) or 3)
    screen.circle(b.x, b.y, b.r)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text(lit and "lavalamp" or "lavalamp (off)")
  screen.level(4)
  screen.move(0, 62)
  screen.text("heat " .. params:string("heat"))
  screen.move(127, 62)
  screen.text_right(QUAL[params:get("qual")])
  screen.update()
end
