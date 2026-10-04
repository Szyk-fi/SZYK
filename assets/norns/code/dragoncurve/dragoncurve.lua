-- dragoncurve
-- a Portamax norns script
--
-- a strip of paper folded in half
-- again and again, then opened:
-- the dragon curve draws itself
-- one fold at a time. a left turn
-- steps the melody up, a right
-- turn steps it down.
--
-- E2 speed   E3 brightness
-- K2 restart   K3 mirror turns
-- pads: transpose to the pad note
-- (params: scale, root, length)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local DX = { 1, 0, -1, 0 }
local DY = { 0, -1, 0, 1 }
local scale = {}
local pts = {}
local n = 0
local dir = 1
local x, y = 0, 0
local minx, maxx, miny, maxy = 0, 0, 0, 0
local degree = 8
local rising = 1
local mirror = false
local last_turn = 0
local flash = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 16)
end

local function restart()
  n = 0
  dir = math.random(1, 4)
  x, y = 0, 0
  pts = { { 0, 0 } }
  minx, maxx, miny, maxy = 0, 0, 0, 0
  degree = 8
end

-- the regular paper-folding sequence: the bit above the lowest
-- set bit of n says which way the n-th fold turns
local function turn_of(k)
  local low = k & -k
  return ((low << 1) & k) ~= 0 and -1 or 1
end

local function unfold()
  if n >= 2 ^ params:get("order") then restart() end
  local turn = 0
  if n > 0 then
    turn = turn_of(n)
    if mirror then turn = -turn end
    dir = (dir - 1 + (turn == 1 and 1 or 3)) % 4 + 1
  end
  n = n + 1
  x, y = x + DX[dir], y + DY[dir]
  pts[#pts + 1] = { x, y }
  minx, maxx = math.min(minx, x), math.max(maxx, x)
  miny, maxy = math.min(miny, y), math.max(maxy, y)

  -- left (1) climbs, right (-1) falls, reflecting at the edges
  degree = degree + turn
  if degree > #scale then degree = #scale - 2 elseif degree < 1 then degree = 3 end
  local note = scale[degree]
  -- a change of direction is a gentle accent
  local accent = turn ~= 0 and turn ~= last_turn
  last_turn = turn
  engine.pan(turn * 0.5)
  engine.pw(accent and 0.2 or 0.45)
  engine.amp(accent and 0.28 or 0.17)
  engine.hz(MusicUtil.note_num_to_freq(note))
  flash = 15
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("DRAGONCURVE")
  params:add_option("scale", "scale", names, 2)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 52, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_number("order", "length (folds)", 6, 11, 9)
  params:add_option("speed", "speed", { "1/2", "1/3", "1/4", "1/6", "1/8" }, 3)
  params:add_control("bright", "brightness", controlspec.new(200, 6000, 'exp', 0, 1600, 'hz'))
  params:set_action("bright", function(v) engine.cutoff(v) end)
  params:add_control("release", "release", controlspec.new(0.05, 3, 'exp', 0, 0.7, 's'))
  params:set_action("release", function(v) engine.release(v) end)
  params:default()
  engine.gain(1.3)
  math.randomseed(os.time())
  build_scale()
  restart()
  unfold()

  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      params:set("root", util.clamp(msg.note - 12, 36, 72))
      engine.pan(0)
      engine.hz(MusicUtil.note_num_to_freq(msg.note))
    end
  end

  clock.run(function()
    local divs = { 1 / 2, 1 / 3, 1 / 4, 1 / 6, 1 / 8 }
    while true do
      clock.sync(divs[params:get("speed")])
      unfold()
    end
  end)
  local mt = metro.init(function()
    flash = math.max(0, flash - 2)
    redraw()
  end, 1 / 15)
  mt:start()
end

function enc(k, d)
  if k == 2 then params:delta("speed", d)
  elseif k == 3 then params:delta("bright", d) end
end

function key(k, z)
  if z == 0 then return end
  if k == 2 then restart()
  elseif k == 3 then mirror = not mirror end
end

function redraw()
  screen.clear()
  -- fit the curve so far into the drawing area
  local w, h = math.max(1, maxx - minx), math.max(1, maxy - miny)
  local s = math.min(118 / w, 48 / h, 8)
  local ox = 64 - (minx + maxx) / 2 * s
  local oy = 30 - (miny + maxy) / 2 * s
  local total = #pts
  -- older folds dim, the newest stay bright
  local seg = 64
  for start = 1, total - 1, seg do
    local age = (total - start) / math.max(1, total)
    screen.level(math.floor(util.linlin(0, 1, 12, 3, age)))
    screen.move(ox + pts[start][1] * s, oy + pts[start][2] * s)
    for i = start + 1, math.min(total, start + seg) do
      screen.line(ox + pts[i][1] * s, oy + pts[i][2] * s)
    end
    screen.stroke()
  end
  screen.level(math.max(8, flash))
  screen.circle(ox + x * s, oy + y * s, 1.5)
  screen.fill()
  screen.level(15)
  screen.move(0, 63)
  screen.text(mirror and "dragoncurve <>" or "dragoncurve")
  screen.level(4)
  screen.move(127, 63)
  screen.text_right(n .. "/" .. math.floor(2 ^ params:get("order")))
  screen.update()
end
