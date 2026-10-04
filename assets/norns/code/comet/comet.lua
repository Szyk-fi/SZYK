-- comet
-- a Portamax norns script
--
-- a comet swings round the sun on
-- an ellipse, obeying Kepler: it
-- races at perihelion and crawls
-- far out. a note sounds every
-- twelfth of the way round, so the
-- notes crowd together near the sun
-- and climb as it falls inward.
--
-- E2 eccentricity   E3 orbit time
-- K2 new comet   K3 pause
-- (params: scale, root, notes/orbit)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local scale = {}
local theta = -0.2
local dir = 1 -- which side perihelion is on
local S = 56
local paused, slot, flash = false, 0, 0
local trail = {}

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 16)
end

-- distance from the sun (the focus) for semi-major axis 1
local function radius(th)
  local e = params:get("ecc")
  return (1 - e * e) / (1 + e * math.cos(th))
end

-- the sun sits at the focus, so the whole ellipse stays on screen
local function sun_x() return 64 + dir * params:get("ecc") * S end

local function pos(th)
  local r = radius(th) * S
  return sun_x() + dir * r * math.cos(th), 33 + r * math.sin(th) * 0.36
end

local function sound()
  local e = params:get("ecc")
  local near = util.linlin(1 - e, 1 + e, 1, 0, radius(theta)) -- 1 at perihelion
  local deg = util.clamp(math.floor(1 + near * 13 + 0.5), 1, #scale)
  local x = pos(theta)
  engine.pan((x - 64) / 64)
  engine.pw(0.2 + 0.5 * near)
  engine.release(0.4 + (1 - near) * 1.8)
  engine.hz(MusicUtil.note_num_to_freq(scale[deg]))
  flash = 15
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("COMET")
  params:add_option("scale", "scale", names, 8)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 48, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("ecc", "eccentricity", controlspec.new(0, 0.9, 'lin', 0.01, 0.7, ''))
  params:add_control("period", "orbit time", controlspec.new(3, 30, 'exp', 0, 8, 's'))
  params:add_number("slots", "notes/orbit", 4, 32, 16)
  params:add_control("bright", "brightness", controlspec.new(300, 6000, 'exp', 0, 2000, 'hz'))
  params:set_action("bright", function(x) engine.cutoff(x) end)
  params:default()
  engine.amp(0.24)
  math.randomseed(os.time())
  build_scale()
  sound()
  midi.connect().event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then engine.release(2.5) engine.hz(MusicUtil.note_num_to_freq(msg.note + 12)) end
  end
  metro.init(function()
    if not paused then step(1 / 30) end
    flash = math.max(0, flash - 1)
    redraw()
  end, 1 / 30):start()
end

function step(dt)
  -- Kepler's second law: dtheta/dt = n (1 + e cos th)^2 / (1 - e^2)^1.5
  local e = params:get("ecc")
  local n = 2 * math.pi / params:get("period")
  theta = theta + n * (1 + e * math.cos(theta)) ^ 2 / (1 - e * e) ^ 1.5 * dt
  if theta > math.pi then theta = theta - 2 * math.pi end
  local s = math.floor((theta + math.pi) / (2 * math.pi) * params:get("slots"))
  if s ~= slot then slot = s sound() end
  table.insert(trail, 1, { pos(theta) })
  trail[15] = nil
end

function enc(n, d)
  if n == 2 then params:delta("ecc", d)
  elseif n == 3 then params:delta("period", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    dir = -dir
    params:set("ecc", 0.4 + math.random() * 0.45)
    theta, trail = -0.3, {}
    sound()
  elseif n == 3 then paused = not paused end
end

function redraw()
  screen.clear()
  screen.level(2)
  for i = 0, 63 do
    local x, y = pos(i / 64 * 2 * math.pi)
    screen.pixel(x, y)
    screen.fill()
  end
  screen.level(15) -- the sun
  screen.circle(sun_x(), 33, 3)
  screen.fill()
  for i, p in ipairs(trail) do
    screen.level(math.max(1, 12 - i))
    screen.pixel(p[1], p[2])
    screen.fill()
  end
  local x, y = pos(theta)
  -- the tail always points away from the sun
  local dx, dy = x - sun_x(), y - 33
  local d = math.max(1, math.sqrt(dx * dx + dy * dy))
  screen.level(6)
  screen.move(x, y)
  local tail = 4 + 300 / (d + 10)
  screen.line(x + dx / d * tail, y + dy / d * tail)
  screen.stroke()
  screen.level(math.max(8, flash))
  screen.circle(x, y, 2)
  screen.fill()
  screen.level(15)
  screen.move(0, 8)
  screen.text(paused and "comet (paused)" or "comet")
  screen.level(4)
  screen.move(0, 62)
  screen.text("e " .. params:string("ecc"))
  screen.move(127, 62)
  screen.text_right("orbit " .. params:string("period"))
  screen.update()
end
