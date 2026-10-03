-- lissajous
-- a Portamax norns script
--
-- two oscillations, one across and
-- one up-down, trace a lissajous
-- figure. each time the dot hits
-- the right edge one voice sounds;
-- each time it hits the top, a
-- second voice sounds at the very
-- same frequency ratio (3:2 is a
-- pure fifth, 5:4 a pure third).
--
-- E2 x ratio   E3 y ratio
-- K2 new melody   K3 figure/trail
-- pads: set the melody note
-- (params: scale, root, rate,
--  brightness, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local CX, CY, RX, RY = 64, 33, 40, 24
local scale = {}
local theta = 0
local phi = 0
local degree = 5
local trail = {}
local show_figure = true
local last_dx, last_dy = 0, 0
local flash_x, flash_y = 0, 0
local cycles = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 12)
end

local function ratio()
  local r = params:get("b") / params:get("a")
  -- fold into one octave above the base, keeping the interval's class
  while r >= 2 do r = r / 2 end
  while r < 1 do r = r * 2 end
  return r
end

local function base_hz()
  return MusicUtil.note_num_to_freq(scale[util.clamp(degree, 1, #scale)])
end

local function voice(hz, pan, pw, amp)
  engine.pan(pan)
  engine.pw(pw)
  engine.amp(amp)
  engine.hz(hz)
end

local function step()
  local a, b = params:get("a"), params:get("b")
  local w = 2 * math.pi * params:get("rate") / 30
  local before = theta
  theta = theta + w
  phi = phi + 0.004
  -- once per full turn the melody takes a step
  if math.floor(theta / (2 * math.pi)) ~= math.floor(before / (2 * math.pi)) then
    cycles = cycles + 1
    local moves = { -2, -1, 1, 1, 2, -1 }
    degree = degree + moves[math.random(1, #moves)]
    if degree < 1 then degree = 3 elseif degree > #scale - 4 then degree = #scale - 6 end
  end
  -- the derivative of each axis changes sign at a peak
  local dx = math.cos(a * theta + phi)
  local dy = math.cos(b * theta)
  if last_dx > 0 and dx <= 0 then
    voice(base_hz(), -0.6, 0.45, 0.22)
    flash_x = 15
  end
  -- y is drawn upward, so its maximum is the top edge
  if last_dy > 0 and dy <= 0 then
    voice(base_hz() * ratio(), 0.6, 0.25, 0.18)
    flash_y = 15
  end
  last_dx, last_dy = dx, dy
  table.insert(trail, { CX + math.sin(a * theta + phi) * RX, CY - math.sin(b * theta) * RY })
  if #trail > 40 then table.remove(trail, 1) end
  flash_x = math.max(0, flash_x - 1)
  flash_y = math.max(0, flash_y - 1)
  redraw()
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("LISSAJOUS")
  params:add_number("a", "x ratio", 1, 7, 2)
  params:add_number("b", "y ratio", 1, 7, 3)
  params:add_option("scale", "scale", names, 11)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 45, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("rate", "rate", controlspec.new(0.1, 2, 'exp', 0, 0.6, 'hz'))
  params:add_control("bright", "brightness", controlspec.new(200, 6000, 'exp', 0, 1700, 'hz'))
  params:set_action("bright", function(v) engine.cutoff(v) end)
  params:add_control("release", "release", controlspec.new(0.1, 4, 'exp', 0, 1.2, 's'))
  params:set_action("release", function(v) engine.release(v) end)
  params:default()
  engine.gain(1.2)
  math.randomseed(os.time())
  build_scale()
  -- open on the interval itself
  voice(base_hz(), -0.6, 0.45, 0.22)
  voice(base_hz() * ratio(), 0.6, 0.25, 0.18)

  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      local deg = 1
      for i, n in ipairs(scale) do if n <= msg.note then deg = i end end
      degree = deg
      voice(MusicUtil.note_num_to_freq(msg.note), 0, 0.4, 0.2)
    end
  end

  local mt = metro.init(step, 1 / 30)
  mt:start()
end

function enc(n, d)
  if n == 2 then params:delta("a", d)
  elseif n == 3 then params:delta("b", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    degree = math.random(2, 7)
    phi = math.random() * math.pi
  elseif n == 3 then
    show_figure = not show_figure
  end
end

function redraw()
  screen.clear()
  local a, b = params:get("a"), params:get("b")
  if show_figure then
    screen.level(3)
    local steps = 160
    for i = 0, steps do
      local th = i / steps * 2 * math.pi
      local x = CX + math.sin(a * th + phi) * RX
      local y = CY - math.sin(b * th) * RY
      if i == 0 then screen.move(x, y) else screen.line(x, y) end
    end
    screen.stroke()
  end
  for i = 2, #trail do
    screen.level(math.floor(util.linlin(1, #trail, 1, 12, i)))
    screen.move(trail[i - 1][1], trail[i - 1][2])
    screen.line(trail[i][1], trail[i][2])
    screen.stroke()
  end
  local p = trail[#trail]
  if p then
    screen.level(15)
    screen.circle(p[1], p[2], 2)
    screen.fill()
  end
  -- the edges that sing
  screen.level(math.max(2, flash_x))
  screen.move(CX + RX + 3, CY - RY)
  screen.line(CX + RX + 3, CY + RY)
  screen.stroke()
  screen.level(math.max(2, flash_y))
  screen.move(CX - RX, CY - RY - 3)
  screen.line(CX + RX, CY - RY - 3)
  screen.stroke()
  screen.level(15)
  screen.move(0, 6)
  screen.text(a .. ":" .. b)
  screen.level(4)
  screen.move(127, 6)
  screen.text_right(MusicUtil.note_num_to_name(scale[util.clamp(degree, 1, #scale)], true))
  screen.update()
end
