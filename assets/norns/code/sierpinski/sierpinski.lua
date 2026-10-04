-- sierpinski
-- a Portamax norns script
--
-- the chaos game: a point leaps
-- part-way toward a random corner,
-- again and again, and a fractal
-- appears out of noise. each
-- corner is a chord tone; the
-- chord moves every few bars.
--
-- E2 density   E3 brightness
-- K2 clear   K3 triangle/pentagon
-- pads: set the chord root
-- (params: scale, root, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local scale = {}
local corners = {}
local shape = 3
local px, py = 64, 30
local dots = {}
local hit = {}
local lit = 0
local last_corner = 1
local beat = 0
local chord_i = 1
-- scale degrees the chord sits on: I vi IV V
local PROGRESSION = { 1, 6, 4, 5 }
-- chord tones above the chord root, in scale steps
local TONES = { 0, 2, 4, 6, 8 }

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 24)
end

local function make_shape()
  corners = {}
  local cx, cy, r = 64, 30, 29
  for i = 1, shape do
    local a = -math.pi / 2 + (i - 1) * 2 * math.pi / shape
    corners[i] = { x = cx + math.cos(a) * r * (shape == 3 and 1.25 or 1), y = cy + math.sin(a) * r + (shape == 3 and 5 or 2), glow = 0 }
  end
  dots = {}
  hit = {}
end

-- how far toward the corner the point jumps; these ratios
-- are what make the gaps in each figure
local function ratio() return shape == 3 and 0.5 or 0.618 end

local function leap()
  local c = math.random(1, shape)
  local k = corners[c]
  px = px + (k.x - px) * ratio()
  py = py + (k.y - py) * ratio()
  -- each pixel is kept once, so the figure fills in rather than piles up
  local x, y = math.floor(px), math.floor(py)
  local key = y * 128 + x
  if not hit[key] then
    hit[key] = true
    dots[#dots + 1] = { x, y }
  end
  return c
end

local function play(c)
  local root_deg = PROGRESSION[chord_i]
  local deg = root_deg + TONES[((c - 1) % #TONES) + 1]
  -- higher on the screen, higher in pitch
  if py < 22 then deg = deg + 7 end
  local note = scale[util.clamp(deg, 1, #scale)]
  local k = corners[c]
  engine.pan(util.linlin(10, 118, -0.8, 0.8, k.x))
  engine.pw(0.3 + 0.1 * c)
  engine.amp(0.2)
  engine.hz(MusicUtil.note_num_to_freq(note))
  k.glow = 15
  last_corner = c
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("SIERPINSKI")
  params:add_option("scale", "scale", names, 1)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 48, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_number("density", "density", 1, 40, 6)
  params:add_number("bars", "beats per chord", 2, 32, 8)
  params:add_control("bright", "brightness", controlspec.new(200, 6000, 'exp', 0, 2200, 'hz'))
  params:set_action("bright", function(x) engine.cutoff(x) end)
  params:add_control("release", "release", controlspec.new(0.1, 4, 'exp', 0, 1.1, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:default()
  engine.gain(1.4)
  math.randomseed(os.time())
  build_scale()
  make_shape()
  play(leap())

  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      chord_i = (msg.note % #PROGRESSION) + 1
      engine.pan(0)
      engine.hz(MusicUtil.note_num_to_freq(msg.note))
    end
  end

  clock.run(function()
    while true do
      clock.sync(1 / 4)
      beat = beat + 1
      if beat % (params:get("bars") * 4) == 0 then chord_i = chord_i % #PROGRESSION + 1 end
      -- the silent leaps draw; the last one of each step sings
      local c
      for _ = 1, params:get("density") do c = leap() end
      play(c)
    end
  end)
  local mt = metro.init(function()
    for _, k in ipairs(corners) do k.glow = math.max(0, k.glow - 1) end
    redraw()
  end, 1 / 15)
  mt:start()
end

function enc(n, d)
  if n == 2 then params:delta("density", d)
  elseif n == 3 then params:delta("bright", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then dots = {} hit = {}
  elseif n == 3 then
    shape = shape == 3 and 5 or 3
    make_shape()
  end
end

function redraw()
  screen.clear()
  screen.level(6)
  for _, d in ipairs(dots) do
    screen.pixel(d[1], d[2])
  end
  screen.fill()
  for i, k in ipairs(corners) do
    screen.level(math.max(3, k.glow))
    screen.circle(k.x, k.y, i == last_corner and 2.5 or 1.5)
    screen.fill()
  end
  screen.level(15)
  screen.pixel(math.floor(px), math.floor(py))
  screen.fill()
  screen.level(15)
  screen.move(0, 7)
  screen.text("sierpinski")
  screen.level(4)
  screen.move(127, 7)
  local names = { "I", "vi", "IV", "V" }
  screen.text_right(names[chord_i])
  screen.move(0, 63)
  screen.text("dens " .. params:get("density"))
  screen.move(127, 63)
  screen.text_right(#dots)
  screen.update()
end
