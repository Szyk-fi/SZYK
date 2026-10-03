-- dunes
-- a Portamax norns script
--
-- wind lifts sand grains off the
-- windward slopes and drops them
-- downwind; steep faces slump, and
-- the dunes slowly migrate. a
-- reading line scans the desert,
-- turning dune heights into a
-- slow melody.
--
-- E2 wind       E3 scan speed
-- K2 new desert K3 wind turns
-- pads: a gust from that column
-- (params: scale, root, scan)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local W = 128
local BASE = 62
local REPOSE = 2
local h = {}
local airborne = {}
local scale = {}
local scan = 1
local dir = 1
local divs = { 1, 1 / 2, 1 / 4 }
local div_names = { "1", "1/2", "1/4" }
local readings = {}
local t = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 16)
end

local function wrap(x) return ((x - 1) % W) + 1 end

local function new_desert()
  local p1, p2 = math.random() * 6, math.random() * 6
  for x = 1, W do
    h[x] = math.floor(12 + 7 * math.sin(x / W * 2 * math.pi * 2 + p1) + 4 * math.sin(x / W * 2 * math.pi * 5 + p2) + math.random() * 2)
    h[x] = math.max(2, h[x])
  end
  airborne = {}
end

-- topple grains down any face steeper than the angle of repose
local function relax(x)
  for _ = 1, 8 do
    local moved = false
    for _, d in ipairs({ dir, -dir }) do
      local n = wrap(x + d)
      if h[x] - h[n] > REPOSE then
        h[x] = h[x] - 1
        h[n] = h[n] + 1
        x = n
        moved = true
        break
      end
    end
    if not moved then return end
  end
end

local function lift(x)
  -- grains leave exposed (windward or crest) cells, not sheltered hollows
  local up = h[wrap(x - dir)]
  if h[x] > 2 and h[x] >= up then
    h[x] = h[x] - 1
    local hop = math.random(3, 6) + math.floor(params:get("wind") * 4)
    table.insert(airborne, { x = x, from = x, hop = hop, y = BASE - h[x], k = 0 })
  end
end

local function read_note()
  local x = math.floor(scan)
  local height = h[wrap(x)]
  -- slope under the reader picks the articulation: slip faces are short
  local slope = h[wrap(x + 1)] - h[wrap(x - 1)]
  local deg = util.clamp(math.floor(util.linlin(2, 30, 1, #scale, height)), 1, #scale)
  engine.pan((x / W - 0.5) * 1.4)
  engine.amp(0.24)
  engine.pw(util.clamp(0.5 + slope * 0.08, 0.1, 0.9))
  engine.release(math.abs(slope) > 2 and 0.4 or 1.8)
  engine.cutoff(params:get("bright"))
  engine.hz(MusicUtil.note_num_to_freq(scale[deg]))
  table.insert(readings, { x = x, y = BASE - height, life = 12 })
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("DUNES")
  params:add_option("scale", "scale", names, 37)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 64, 45, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("wind", "wind", controlspec.new(0, 1, 'lin', 0, 0.4, ''))
  params:add_option("div", "scan step", div_names, 2)
  params:add_control("bright", "brightness", controlspec.new(300, 8000, 'exp', 0, 1500, 'hz'))
  params:default()
  engine.gain(1.2)
  math.randomseed(os.time())
  build_scale()
  new_desert()
  midi.connect().event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      local x0 = (msg.note * 11) % W + 1
      for k = 0, 6 do lift(wrap(x0 + k)) end
    end
  end
  local m = metro.init(step, 1 / 30)
  m:start()
  -- the reader: one note per step, synced to the clock
  clock.run(function()
    while true do
      read_note()
      scan = wrap(math.floor(scan) + 3)
      clock.sync(divs[params:get("div")])
    end
  end)
end

function step()
  t = t + 1 / 30
  local wind = params:get("wind")
  local lifts = math.floor(wind * 10 + math.random())
  for _ = 1, lifts do lift(math.random(1, W)) end
  for i = #airborne, 1, -1 do
    local g = airborne[i]
    g.k = g.k + 1
    local frac = g.k / 8
    g.x = wrap(g.from + math.floor(g.hop * frac * dir + 0.5))
    g.y = BASE - h[wrap(g.from)] - math.sin(frac * math.pi) * (4 + wind * 6)
    if g.k >= 8 then
      local x = wrap(g.from + g.hop * dir)
      h[x] = h[x] + 1
      relax(x)
      table.remove(airborne, i)
    end
  end
  for i = #readings, 1, -1 do
    readings[i].life = readings[i].life - 1
    if readings[i].life <= 0 then table.remove(readings, i) end
  end
  redraw()
end

function enc(n, d)
  if n == 2 then params:delta("wind", d)
  elseif n == 3 then params:delta("div", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_desert()
  elseif n == 3 then dir = -dir end
end

function redraw()
  screen.clear()
  screen.line_width(1)
  -- sky shading: a dim sun
  screen.level(2)
  screen.circle(108, 18, 5)
  screen.fill()
  -- the sand, filled to the base with a lit crest line
  for x = 1, W do
    local top = BASE - h[x]
    local lee = (h[x] - h[wrap(x + dir)]) > 1
    screen.level(lee and 2 or 4)
    screen.move(x - 0.5, 64)
    screen.line(x - 0.5, top + 1)
    screen.stroke()
    screen.level(lee and 6 or 11)
    screen.pixel(x - 1, top)
    screen.fill()
  end
  screen.level(13)
  for _, g in ipairs(airborne) do
    screen.pixel(g.x - 1, math.floor(g.y))
    screen.fill()
  end
  -- the reading line
  local sx = math.floor(scan) - 0.5
  screen.level(7)
  screen.move(sx, 12)
  screen.line(sx, BASE - h[wrap(math.floor(scan))] - 2)
  screen.stroke()
  for _, r in ipairs(readings) do
    screen.level(r.life)
    screen.circle(r.x, r.y, 2 + (12 - r.life) * 0.3)
    screen.stroke()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text("dunes")
  screen.level(4)
  screen.move(127, 8)
  screen.text_right((dir > 0 and "wind > " or "< wind ") .. div_names[params:get("div")])
  screen.update()
end
