-- hilbert
-- a Portamax norns script
--
-- a cursor crawls a hilbert curve,
-- visiting every cell of a square
-- without ever jumping. across is
-- pitch, up and down is pan: a
-- melody that wanders but never
-- leaps.
--
-- E2 speed   E3 brightness
-- K2 next order   K3 reverse
-- pads: re-root the scale
-- (params: order 2-5, scale, root)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local scale = {}
local path = {}
local N = 8
local pos = 1
local dirn = 1
local flash = 0
local X0, Y0, SIZE = 4, 4, 56

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 16)
end

-- distance along the curve -> cell, the classic iterative form
local function d2xy(n, d)
  local x, y, t, s = 0, 0, d, 1
  while s < n do
    local rx = (t // 2) & 1
    local ry = (t ~ rx) & 1
    if ry == 0 then
      if rx == 1 then x, y = s - 1 - x, s - 1 - y end
      x, y = y, x
    end
    x, y = x + s * rx, y + s * ry
    t = t // 4
    s = s * 2
  end
  return x, y
end

local function build_path()
  N = 1 << params:get("order")
  path = {}
  for d = 0, N * N - 1 do
    local x, y = d2xy(N, d)
    path[d + 1] = { x, y }
  end
  pos = util.clamp(pos, 1, #path)
end

local function cell_xy(c)
  local step = SIZE / N
  return X0 + (c[1] + 0.5) * step, Y0 + (c[2] + 0.5) * step
end

local function advance()
  local prev = path[pos]
  pos = pos + dirn
  if pos > #path then pos = 1 elseif pos < 1 then pos = #path end
  local c = path[pos]
  local deg = math.floor(util.linlin(0, N - 1, 1, #scale, c[1]) + 0.5)
  local vertical = prev and prev[1] == c[1]
  engine.pan(util.linlin(0, N - 1, -0.9, 0.9, c[2]))
  engine.pw(util.linlin(0, N - 1, 0.15, 0.6, c[2]))
  -- sideways steps change the note and speak up; vertical
  -- steps repeat it softly while it travels across the field
  engine.amp(vertical and 0.12 or 0.24)
  engine.hz(MusicUtil.note_num_to_freq(scale[util.clamp(deg, 1, #scale)]))
  flash = 15
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("HILBERT")
  params:add_number("order", "order", 2, 5, 3)
  params:set_action("order", build_path)
  params:add_option("scale", "scale", names, 8)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 50, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_option("speed", "speed", { "1/2", "1/3", "1/4", "1/6", "1/8" }, 3)
  params:add_control("bright", "brightness", controlspec.new(200, 6000, 'exp', 0, 1400, 'hz'))
  params:set_action("bright", function(v) engine.cutoff(v) end)
  params:add_control("release", "release", controlspec.new(0.05, 3, 'exp', 0, 0.6, 's'))
  params:set_action("release", function(v) engine.release(v) end)
  params:default()
  engine.gain(1.5)
  build_scale()
  build_path()
  advance()

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
      advance()
    end
  end)
  local mt = metro.init(function()
    flash = math.max(0, flash - 2)
    redraw()
  end, 1 / 15)
  mt:start()
end

function enc(n, d)
  if n == 2 then params:delta("speed", d)
  elseif n == 3 then params:delta("bright", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    local o = params:get("order") + 1
    params:set("order", o > 5 and 2 or o)
  elseif n == 3 then
    dirn = -dirn
  end
end

function redraw()
  screen.clear()
  -- the whole curve, dim
  screen.level(2)
  local x, y = cell_xy(path[1])
  screen.move(x, y)
  for i = 2, #path do
    x, y = cell_xy(path[i])
    screen.line(x, y)
  end
  screen.stroke()
  -- a short bright tail behind the cursor
  local tail = math.min(#path - 1, 12)
  for k = tail, 1, -1 do
    local a = (pos - 1 - k * dirn) % #path + 1
    local b = (pos - 1 - (k - 1) * dirn) % #path + 1
    screen.level(math.floor(util.linlin(1, tail + 1, 12, 3, k)))
    local ax, ay = cell_xy(path[a])
    local bx, by = cell_xy(path[b])
    screen.move(ax, ay)
    screen.line(bx, by)
    screen.stroke()
  end
  local c = path[pos]
  local cx, cy = cell_xy(c)
  screen.level(math.max(10, flash))
  screen.circle(cx, cy, 2)
  screen.fill()
  -- pitch and pan readouts
  screen.level(1)
  screen.rect(X0, Y0, SIZE, SIZE)
  screen.stroke()
  screen.level(15)
  screen.move(68, 10)
  screen.text("hilbert")
  screen.level(5)
  screen.move(68, 22)
  screen.text("order " .. params:get("order"))
  local deg = math.floor(util.linlin(0, N - 1, 1, #scale, c[1]) + 0.5)
  screen.move(68, 32)
  screen.text(MusicUtil.note_num_to_name(scale[util.clamp(deg, 1, #scale)], true))
  screen.move(68, 42)
  screen.text(pos .. "/" .. #path)
  -- pan meter
  screen.level(3)
  screen.move(68, 52)
  screen.line(124, 52)
  screen.stroke()
  screen.level(12)
  local px = util.linlin(0, N - 1, 68, 124, c[2])
  screen.rect(px - 1, 50, 3, 5)
  screen.fill()
  screen.level(4)
  screen.move(68, 63)
  screen.text(dirn > 0 and "fwd" or "rev")
  screen.move(127, 63)
  screen.text_right(params:string("speed"))
  screen.update()
end
