-- arcarp
-- a Portamax norns script
--
-- an arpeggio you shape on the arc:
-- ring 1 speed, ring 2 transpose,
-- ring 3 brightness, ring 4 length
-- of each note. push any ring for
-- a new pattern.
--
-- E2/E3 also turn rings 1/2
-- K3 new pattern
-- open the Arc app to turn the rings

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local a = arc.connect()
-- each ring's value, 0..1
local v = { 0.5, 0.5, 0.5, 0.35 }
local pattern = {}
local step = 0
local last = 0
local DIVS = { 1, 1/2, 1/3, 1/4, 1/6, 1/8 }

local function new_pattern()
  for i = 1, 8 do pattern[i] = math.random(0, 7) end
end

local function div() return DIVS[math.floor(v[1] * (#DIVS - 1) + 0.5) + 1] end
local function transpose() return math.floor(v[2] * 24 + 0.5) - 12 end
local function cutoff() return 300 * (2 ^ (v[3] * 5)) end
local function release() return 0.05 + v[4] * 1.5 end

function arc_redraw()
  a:all(0)
  for n = 1, 4 do
    -- a bright line along the value, from the top round to it
    local to = v[n] * 2 * math.pi * 0.999
    a:segment(n, 0, to, 6)
    a:led(n, math.floor(v[n] * 63) + 1, 15)
  end
  -- the note that just played, on ring 2's far side
  a:led(2, 33 + last % 8, 10)
  a:refresh()
end

a.delta = function(n, d)
  v[n] = util.clamp(v[n] + d / 512, 0, 1)
  engine.cutoff(cutoff())
  engine.release(release())
  arc_redraw()
  redraw()
end

a.key = function(n, z)
  if z == 1 then new_pattern() arc_redraw() end
end

function init()
  math.randomseed(7)
  new_pattern()
  engine.amp(0.3)
  engine.cutoff(cutoff())
  engine.release(release())
  clock.run(function()
    while true do
      clock.sync(div())
      step = step % #pattern + 1
      local degree = pattern[step]
      local scale = MusicUtil.generate_scale_of_length(48 + transpose(), 1, 8)
      last = degree
      engine.hz(MusicUtil.note_num_to_freq(scale[degree + 1]))
      arc_redraw()
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then a.delta(1, d * 8)
  elseif n == 3 then a.delta(2, d * 8) end
end

function key(n, z)
  if n == 3 and z == 1 then new_pattern() arc_redraw() end
end

function redraw()
  screen.clear()
  local names = { "speed", "transp", "bright", "length" }
  for n = 1, 4 do
    local x = 4 + (n - 1) * 32
    screen.level(3)
    screen.circle(x + 12, 32, 11)
    screen.stroke()
    screen.level(15)
    local ang = v[n] * 2 * math.pi
    screen.move(x + 12, 32)
    screen.line(x + 12 + 11 * math.sin(ang), 32 - 11 * math.cos(ang))
    screen.stroke()
    screen.level(6)
    screen.move(x + 12, 56)
    screen.text_center(names[n])
  end
  screen.level(15)
  screen.move(0, 7)
  screen.text("arcarp")
  screen.level(4)
  screen.move(128, 7)
  screen.text_right(a.name)
  screen.update()
end
