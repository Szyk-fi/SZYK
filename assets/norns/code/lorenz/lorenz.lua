-- lorenz
-- a Portamax norns script
--
-- the lorenz attractor: a point
-- that circles two lobes and never
-- repeats. its x is pan, its y is
-- the note, its z is brightness.
--
-- E2 speed   E3 rho (chaos)
-- K2 nudge   K3 rest / go
-- (params: scale, root)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local x, y, z = 0.1, 0, 0
local trail = {}
local scale = {}
local resting = false

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 24)
end

local function advance(dt)
  local sigma, beta, rho = 10, 8 / 3, params:get("rho")
  for _ = 1, 4 do
    local dx = sigma * (y - x)
    local dy = x * (rho - z) - y
    local dz = x * y - beta * z
    x, y, z = x + dx * dt, y + dy * dt, z + dz * dt
  end
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("LORENZ")
  params:add_control("speed", "speed", controlspec.new(0.001, 0.01, 'exp', 0, 0.004, ''))
  params:add_control("rho", "rho", controlspec.new(14, 40, 'lin', 0, 28, ''))
  params:add_option("scale", "scale", names, 6)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 60, 41, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:default()
  engine.release(0.7)
  engine.amp(0.22)
  build_scale()
  local m = metro.init(function()
    if not resting then advance(params:get("speed")) end
    table.insert(trail, { x, z })
    while #trail > 120 do table.remove(trail, 1) end
    redraw()
  end, 1 / 30)
  m:start()
  clock.run(function()
    while true do
      clock.sync(1 / 4)
      if not resting then
        local deg = util.clamp(math.floor((y + 30) / 60 * 23) + 1, 1, 24)
        engine.pan(util.clamp(x / 20, -1, 1))
        engine.cutoff(util.linexp(0, 50, 300, 6000, util.clamp(z, 0, 50)))
        engine.hz(MusicUtil.note_num_to_freq(scale[deg]))
      end
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("speed", d)
  elseif n == 3 then params:delta("rho", d) end
end

function key(n, z_)
  if z_ == 0 then return end
  if n == 2 then x = x + (math.random() - 0.5) * 4 elseif n == 3 then resting = not resting end
end

function redraw()
  screen.clear()
  for i = 2, #trail do
    screen.level(math.floor(1 + i / #trail * 14))
    screen.move(64 + trail[i - 1][1] * 2.4, 66 - trail[i - 1][2] * 1.1)
    screen.line(64 + trail[i][1] * 2.4, 66 - trail[i][2] * 1.1)
    screen.stroke()
  end
  screen.level(15)
  screen.move(0, 7)
  screen.text(resting and "lorenz (rest)" or "lorenz")
  screen.level(4)
  screen.move(127, 7)
  screen.text_right("rho " .. params:string("rho"))
  screen.update()
end
