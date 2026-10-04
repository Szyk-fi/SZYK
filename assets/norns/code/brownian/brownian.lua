-- brownian
-- a Portamax norns script
--
-- a melody that drifts like a
-- particle in water: each note is a
-- small random step from the last.
-- the walls are soft: the nearer it
-- wanders to an edge, the harder it
-- is pushed back toward the middle.
--
-- E2 step size   E3 wall width
-- K2 back to centre   K3 pause
-- pads: centre note
-- (params: scale, rests, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local pos = 0.0 -- in scale degrees from the centre
local trail = {}
local scale = {}
local centre = 60
local paused = false
local playing = false

local function build_scale()
  -- 4 octaves either side, indexed so that 0 is the centre note
  local s = MusicUtil.generate_scale_of_length(centre - 48, params:get("scale"), 96)
  scale = {}
  local mid = 1
  for i, n in ipairs(s) do if n <= centre then mid = i end end
  for i, n in ipairs(s) do scale[i - mid] = n end
end

local function gauss()
  -- sum of uniforms: near enough to a bell curve for a step
  return (math.random() + math.random() + math.random() - 1.5) * 2
end

local function tick()
  local wall = params:get("walls")
  -- a pull toward the centre that grows as the cube of the distance
  local push = -(pos / wall) ^ 3 * 1.5
  pos = util.clamp(pos + gauss() * params:get("step") + push, -wall * 1.3, wall * 1.3)
  table.insert(trail, 1, pos)
  if #trail > 64 then table.remove(trail) end
  playing = #trail < 4 or math.random() > params:get("rests")
  if not playing then return end
  local deg = util.round(pos)
  local note = scale[deg] or centre
  engine.amp(util.linlin(0, wall, 0.3, 0.18, math.abs(pos)))
  engine.pan(util.clamp(pos / wall, -1, 1) * 0.6)
  engine.hz(MusicUtil.note_num_to_freq(note))
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("BROWNIAN")
  params:add_control("step", "step size", controlspec.new(0.2, 4, 'lin', 0.05, 1.0, 'deg'))
  params:add_number("walls", "wall width", 3, 14, 7)
  params:add_option("scale", "scale", names, 5)
  params:set_action("scale", build_scale)
  params:add_control("rests", "rests", controlspec.new(0, 0.6, 'lin', 0.01, 0.15, ''))
  params:add_control("release", "release", controlspec.new(0.1, 3, 'exp', 0, 0.9, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:default()
  engine.cutoff(1900)
  engine.pw(0.5)
  math.randomseed(os.time())
  build_scale()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then centre = msg.note build_scale() end
  end
  metro.init(redraw, 1 / 15):start()
  clock.run(function()
    while true do
      if not paused then tick() end
      -- mostly eighths, now and then a longer breath
      clock.sync(math.random() < 0.8 and 1 / 2 or 1)
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("step", d)
  elseif n == 3 then params:delta("walls", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then pos = 0
  elseif n == 3 then paused = not paused end
  redraw()
end

function redraw()
  screen.clear()
  local wall = params:get("walls")
  local sy = 18 / (wall * 1.3)
  -- the soft walls, shaded as they get firmer
  for i = 1, 4 do
    screen.level(i)
    local off = 34 - (wall * (0.7 + i * 0.15)) * sy
    screen.move(0, off) screen.line(127, off) screen.stroke()
    off = 34 + (wall * (0.7 + i * 0.15)) * sy
    screen.move(0, off) screen.line(127, off) screen.stroke()
  end
  -- the path so far, newest at the right
  screen.level(8)
  for i = 2, #trail do
    screen.move(126 - (i - 2) * 2, 34 - trail[i - 1] * sy)
    screen.line(126 - (i - 1) * 2, 34 - trail[i] * sy)
    screen.stroke()
  end
  if trail[1] then
    screen.level(playing and 15 or 5)
    screen.circle(126, 34 - trail[1] * sy, 2)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text("brownian")
  screen.level(4)
  screen.move(0, 62)
  screen.text("step " .. params:string("step"))
  screen.move(127, 62)
  screen.text_right(paused and "paused" or MusicUtil.note_num_to_name(scale[util.round(pos)] or centre, true))
  screen.update()
end
