-- pendulums
-- a Portamax norns script
--
-- a pendulum wave: twelve
-- pendulums, each a little
-- slower than the last. they
-- drift apart, make patterns, and
-- line up again. each one rings
-- as it swings through the middle.
--
-- E2 cycle length   E3 release
-- K2 restart   K3 mute odd ones
-- (params: scale, root)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local N = 12
-- start just before they all line up, so the piece opens on a chord
local t = -0.6
local last = {}
local scale = {}
local odd_muted = false

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), N)
end

local function angle(i)
  -- pendulum i completes (20 + i) swings per cycle
  local swings = 20 + i
  return math.sin(2 * math.pi * swings * t / params:get("cycle")) * 0.9
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("PENDULUMS")
  params:add_control("cycle", "cycle", controlspec.new(20, 180, 'lin', 1, 60, 's'))
  params:add_control("release", "release", controlspec.new(0.1, 4, 'exp', 0, 1.0, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:add_option("scale", "scale", names, 1)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 55, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:default()
  engine.amp(0.18)
  engine.cutoff(2500)
  build_scale()
  for i = 1, N do last[i] = angle(i) end
  local m = metro.init(function()
    t = t + 1 / 40
    for i = 1, N do
      local a = angle(i)
      if (a >= 0) ~= (last[i] >= 0) and not (odd_muted and i % 2 == 1) then
        engine.pan(i / (N / 2) - 1.08)
        engine.hz(MusicUtil.note_num_to_freq(scale[i]))
      end
      last[i] = a
    end
    redraw()
  end, 1 / 40)
  m:start()
end

function enc(n, d)
  if n == 2 then params:delta("cycle", d)
  elseif n == 3 then params:delta("release", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then t = -0.6 elseif n == 3 then odd_muted = not odd_muted end
end

function redraw()
  screen.clear()
  -- seen from the side, all hung from one point: the longer the
  -- string, the slower the swing
  for i = 1, N do
    local a = angle(i)
    local len = 12 + i * 3.6
    local bx = 64 + math.sin(a) * len * 1.3
    local by = 10 + math.cos(a) * len
    screen.level(2)
    screen.move(64, 10)
    screen.line(bx, by)
    screen.stroke()
    screen.level((odd_muted and i % 2 == 1) and 3 or 15)
    screen.circle(bx, by, 2)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 7)
  screen.text("pendulums")
  screen.level(4)
  screen.move(127, 7)
  screen.text_right(string.format("%.0fs", t % params:get("cycle")))
  screen.update()
end
