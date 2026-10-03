-- orbits
-- a Portamax norns script
--
-- four moons circle a planet at
-- different speeds. each time a
-- moon crosses the top it rings.
--
-- E2 speed   E3 brightness
-- K2 new orbits   K3 freeze
-- (params: scale, root)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local moons = {}
local scale = {}
local frozen = false
local flashes = {}

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 16)
end

local function new_orbits()
  moons = {}
  for i = 1, 4 do
    local dir = i % 2 == 0 and -1 or 1
    moons[i] = {
      -- each starts a little before the top, so they enter one by one
      angle = (-dir * i * 0.25) % (2 * math.pi),
      speed = (0.02 + math.random() * 0.05) * dir,
      r = 6 + i * 6,
      degree = math.random(1, 12),
    }
  end
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("ORBITS")
  params:add_option("scale", "scale", names, 11)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 48, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("speed", "speed", controlspec.new(0.1, 4, 'exp', 0, 1, 'x'))
  params:add_control("bright", "brightness", controlspec.new(300, 8000, 'exp', 0, 2000, 'hz'))
  params:set_action("bright", function(x) engine.cutoff(x) end)
  params:default()
  engine.release(2.2)
  engine.amp(0.3)
  math.randomseed(os.time())
  build_scale()
  new_orbits()
  local m = metro.init(step, 1 / 30)
  m:start()
end

function step()
  if not frozen then
    local k = params:get("speed")
    for i, moon in ipairs(moons) do
      local before = moon.angle
      moon.angle = (moon.angle + moon.speed * k) % (2 * math.pi)
      -- crossing the top (angle 0) in either direction
      local crossed = (moon.speed > 0 and moon.angle < before) or (moon.speed < 0 and moon.angle > before)
      if crossed then
        engine.pan((i - 2.5) / 2)
        engine.pw(0.2 + i * 0.15)
        engine.hz(MusicUtil.note_num_to_freq(scale[moon.degree + i]))
        flashes[i] = 15
      end
    end
  end
  for i = 1, 4 do flashes[i] = math.max(0, (flashes[i] or 0) - 1) end
  redraw()
end

function enc(n, d)
  if n == 2 then params:delta("speed", d)
  elseif n == 3 then params:delta("bright", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_orbits() elseif n == 3 then frozen = not frozen end
end

function redraw()
  screen.clear()
  local cx, cy = 64, 34
  screen.level(15)
  screen.circle(cx, cy, 3)
  screen.fill()
  for i, moon in ipairs(moons) do
    screen.level(2)
    screen.circle(cx, cy, moon.r)
    screen.stroke()
    local x = cx + math.sin(moon.angle) * moon.r
    local y = cy - math.cos(moon.angle) * moon.r
    screen.level(math.max(5, flashes[i] or 0))
    screen.circle(x, y, 2)
    screen.fill()
  end
  screen.level(8)
  screen.move(cx, 2)
  screen.line(cx, 6)
  screen.stroke()
  screen.level(15)
  screen.move(0, 8)
  screen.text(frozen and "orbits (frozen)" or "orbits")
  screen.level(4)
  screen.move(127, 62)
  screen.text_right(params:string("speed"))
  screen.update()
end
