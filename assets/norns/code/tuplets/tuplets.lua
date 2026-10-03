-- tuplets
-- a Portamax norns script
--
-- one bar, split three ways at
-- once: into 3, into 4 and into 5
-- (or any counts you choose). the
-- three voices share only the
-- downbeat unless the counts share
-- a factor; wherever two or more
-- land together, the hit is
-- accented and the chord changes.
--
-- E1 bar length   E2 pick lane
-- E3 lane's count
-- K2 back to 3:4:5   K3 pause
-- pads: root note

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local PROG = { { 0, "minor 7" }, { 8, "major 7" }, { 3, "major 7" }, { 10, "dominant 7" } }
local lanes = { 3, 4, 5 }
local sel = 1
local tick_n = 0
local ticks = 60
local chord_i = 1
local chord = {}
local flash = { 0, 0, 0 }
local counts = { 0, 0, 0 }
local root = 45
local paused = false

local function gcd(a, b) while b > 0 do a, b = b, a % b end return a end
local function lcm(a, b) return a // gcd(a, b) * b end

local function rebuild()
  ticks = lcm(lcm(lanes[1], lanes[2]), lanes[3])
  tick_n = tick_n % ticks
end

local function build_chord()
  local c = PROG[chord_i]
  chord = MusicUtil.generate_chord(root + c[1], c[2], 0)
end

local function tick()
  local hits = {}
  for i = 1, 3 do
    if tick_n % (ticks // lanes[i]) == 0 then hits[#hits + 1] = i end
  end
  local meet = #hits >= 2
  if meet and tick_n > 0 or tick_n == 0 then
    chord_i = tick_n == 0 and (chord_i % #PROG + 1) or chord_i
    build_chord()
  end
  for _, i in ipairs(hits) do
    local k = tick_n // (ticks // lanes[i])
    counts[i] = k + 1
    local note
    if i == 1 then note = chord[1] - 12
    elseif i == 2 then note = chord[(k % #chord) + 1]
    else note = chord[((lanes[3] - k) % #chord) + 1] + 12 end
    engine.amp(meet and 0.3 or 0.17)
    engine.pw(meet and 0.5 or 0.3)
    engine.pan((i - 2) * 0.6)
    engine.hz(MusicUtil.note_num_to_freq(note))
    flash[i] = meet and 15 or 10
  end
  tick_n = (tick_n + 1) % ticks
end

function init()
  params:add_separator("TUPLETS")
  params:add_number("bar", "bar length (beats)", 2, 8, 4)
  params:add_control("release", "release", controlspec.new(0.1, 2, 'exp', 0, 0.6, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:add_control("cutoff", "cutoff", controlspec.new(300, 8000, 'exp', 0, 1800, 'hz'))
  params:set_action("cutoff", function(x) engine.cutoff(x) end)
  params:default()
  rebuild()
  build_chord()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then root = msg.note - 12 build_chord() end
  end
  clock.run(function()
    while true do
      if not paused then tick() end
      clock.sync(params:get("bar") / ticks)
    end
  end)
  metro.init(function()
    for i = 1, 3 do flash[i] = math.max(0, flash[i] - 2) end
    redraw()
  end, 1 / 15):start()
end

function enc(n, d)
  if n == 1 then params:delta("bar", d)
  elseif n == 2 then sel = util.clamp(sel + d, 1, 3)
  elseif n == 3 then lanes[sel] = util.clamp(lanes[sel] + d, 2, 9) rebuild() end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then lanes = { 3, 4, 5 } rebuild()
  elseif n == 3 then paused = not paused end
  redraw()
end

function redraw()
  screen.clear()
  local x0, w = 14, 110
  for i = 1, 3 do
    local y = 10 + i * 12
    screen.level(i == sel and 15 or 4)
    screen.move(0, y + 3)
    screen.text(tostring(lanes[i]))
    screen.level(2)
    screen.move(x0, y) screen.line(x0 + w, y) screen.stroke()
    for k = 0, lanes[i] - 1 do
      local x = x0 + k / lanes[i] * w
      local lit = k + 1 == counts[i]
      screen.level(lit and math.max(5, flash[i]) or 5)
      screen.rect(x - 1, y - (lit and 3 or 2), 3, lit and 7 or 5)
      screen.fill()
    end
  end
  -- the playhead
  local px = x0 + tick_n / ticks * w
  screen.level(8)
  screen.move(px, 14) screen.line(px, 52) screen.stroke()
  screen.level(15)
  screen.move(0, 8)
  screen.text("tuplets")
  screen.level(4)
  screen.move(0, 62)
  screen.text(lanes[1] .. ":" .. lanes[2] .. ":" .. lanes[3] .. "  over " .. params:get("bar"))
  screen.move(127, 62)
  screen.text_right(paused and "paused" or MusicUtil.note_num_to_name(chord[1], false) .. " " .. PROG[chord_i][2])
  screen.update()
end
