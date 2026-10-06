-- scrubber
-- a Portamax norns script
-- after plates by tehn (monome
-- community): LFOs you can grab and
-- scrub by hand.
--
-- each ring is an LFO, its shape
-- drawn round the ring and a bright
-- dot where it is now. turn a ring to
-- push its LFO along or drag it back;
-- push a ring to hold it still (push
-- again to let go).
-- LFO 1 picks the notes, 2 the tone,
-- 3 the note length, 4 how loud.
--
-- E2 speed of all  E3 root
-- K3 let every LFO go
-- open the Arc app to turn the rings

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local a = arc.connect()
local TAU = 2 * math.pi
local phase = { 0, 0.25, 0.5, 0.75 }
local rate = { 1 / 7, 1 / 11, 1 / 5, 1 / 13 }   -- turns per second
local hold = { false, false, false, false }
local speed = 1
local root = 45
local scale = {}

local function lfo(n) return 0.5 + 0.5 * math.sin(phase[n] * TAU) end

function arc_redraw()
  for n = 1, 4 do
    -- the wave, laid round the ring
    for i = 1, 64 do
      local w = 0.5 + 0.5 * math.sin(((i - 1) / 64) * TAU)
      a:led(n, i, hold[n] and 1 or math.floor(w * 4))
    end
    a:led(n, math.floor((phase[n] % 1) * 64) + 1, 15)
  end
  a:refresh()
end

a.delta = function(n, d)
  -- a step is a 1024th of a turn, as on the hardware
  phase[n] = (phase[n] + d / 1024) % 1
  arc_redraw()
end

a.key = function(n, z)
  if z == 1 then hold[n] = not hold[n] end
  arc_redraw()
end

function init()
  scale = MusicUtil.generate_scale_of_length(root, 2, 24)
  engine.amp(0.3)
  clock.run(function()
    local dt = 1 / 30
    while true do
      clock.sleep(dt)
      for n = 1, 4 do
        if not hold[n] then phase[n] = (phase[n] + rate[n] * speed * dt) % 1 end
      end
      arc_redraw()
    end
  end)
  clock.run(function()
    while true do
      clock.sync(1 / 4)
      engine.cutoff(400 * 2 ^ (lfo(2) * 4))
      engine.release(0.08 + lfo(3) * 1.2)
      engine.amp(0.08 + lfo(4) * 0.35)
      local degree = 1 + math.floor(lfo(1) * 15)
      engine.hz(MusicUtil.note_num_to_freq(scale[degree]))
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then speed = util.clamp(speed + d / 10, 0, 8)
  elseif n == 3 then
    root = util.clamp(root + d, 30, 66)
    scale = MusicUtil.generate_scale_of_length(root, 2, 24)
  end
  redraw()
end

function key(n, z)
  if n == 3 and z == 1 then hold = { false, false, false, false } end
end

function redraw()
  screen.clear()
  local names = { "note", "tone", "length", "level" }
  for n = 1, 4 do
    local y = 14 + (n - 1) * 12
    screen.level(hold[n] and 15 or 4)
    screen.move(0, y + 4)
    screen.text(names[n])
    -- a little trace of the wave with the dot on it
    screen.level(2)
    for x = 0, 80, 2 do
      screen.pixel(40 + x, y + 2 - math.floor(4 * math.sin((x / 80) * TAU)))
    end
    screen.fill()
    screen.level(15)
    local px = 40 + (phase[n] % 1) * 80
    screen.circle(px, y + 2 - 4 * math.sin(phase[n] * TAU), 2)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 7)
  screen.text("scrubber")
  screen.level(4)
  screen.move(128, 7)
  screen.text_right(string.format("x%.1f", speed))
  screen.update()
end
