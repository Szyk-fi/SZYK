-- shoals
-- a Portamax norns script
-- after Shoal by JP (monome
-- community): a chaotic oscillator
-- you can push around.
--
-- four fish swim the arc's rings,
-- each one a chaotic map tugging on
-- its neighbours. each step one fish
-- sings its place as a note.
-- ring 1 chaos, ring 2 schooling
-- (how much they follow each other),
-- ring 3 range, ring 4 how often
-- they sing. push a ring to startle
-- its fish.
--
-- E2/E3 turn rings 1/2
-- K3 startle them all
-- open the Arc app to turn the rings

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local a = arc.connect()
local fish = { 0.21, 0.47, 0.66, 0.83 }
local trail = { {}, {}, {}, {} }
-- the rings' values, 0..1
local v = { 0.7, 0.3, 0.5, 0.7 }
local turn = 0
local scale = {}

local function chaos() return 3.55 + v[1] * 0.45 end

local function f(x) return chaos() * x * (1 - x) end

local function swim()
  local c = v[2] * 0.5
  local nxt = {}
  for i = 1, 4 do
    local l, r = fish[(i - 2) % 4 + 1], fish[i % 4 + 1]
    nxt[i] = util.clamp((1 - c) * f(fish[i]) + c / 2 * (f(l) + f(r)), 0.001, 0.999)
  end
  for i = 1, 4 do
    table.insert(trail[i], 1, fish[i])
    if #trail[i] > 6 then table.remove(trail[i]) end
    fish[i] = nxt[i]
  end
end

local function sing()
  turn = turn % 4 + 1
  if math.random() > 0.15 + v[4] * 0.85 then return end
  local span = 4 + math.floor(v[3] * 20)
  local degree = 1 + math.floor(fish[turn] * span)
  engine.amp(0.15 + 0.2 * fish[turn])
  engine.hz(MusicUtil.note_num_to_freq(scale[util.clamp(degree, 1, #scale)]))
end

function arc_redraw()
  a:all(0)
  for n = 1, 4 do
    -- the ring's setting, dim, from the top
    a:segment(n, 0, v[n] * 2 * math.pi * 0.999, 2)
    -- the fish and its fading trail
    for k, x in ipairs(trail[n]) do
      a:led(n, math.floor(x * 64) + 1, math.max(12 - k * 2, 1))
    end
    a:led(n, math.floor(fish[n] * 64) + 1, 15)
  end
  a:refresh()
end

a.delta = function(n, d)
  v[n] = util.clamp(v[n] + d / 600, 0, 1)
  arc_redraw()
  redraw()
end

a.key = function(n, z)
  if z == 1 then fish[n] = math.random() * 0.98 + 0.01 end
end

function init()
  scale = MusicUtil.generate_scale_of_length(38, 5, 40)
  engine.release(0.7)
  engine.cutoff(1600)
  clock.run(function()
    while true do
      clock.sync(1 / 4)
      swim()
      sing()
      arc_redraw()
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then a.delta(1, d * 12) elseif n == 3 then a.delta(2, d * 12) end
end

function key(n, z)
  if n == 3 and z == 1 then
    for i = 1, 4 do a.key(i, 1) end
  end
end

function redraw()
  screen.clear()
  -- the shoal: each fish against the one before it
  for i = 1, 4 do
    screen.level(i == turn and 15 or 5)
    local x = 20 + fish[i] * 88
    local y = 14 + fish[(i - 2) % 4 + 1] * 38
    screen.circle(x, y, i == turn and 3 or 2)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 7)
  screen.text("shoals")
  screen.level(4)
  screen.move(128, 7)
  screen.text_right(string.format("r %.2f", chaos()))
  screen.move(0, 62)
  screen.text(string.format("school %d%%  sing %d%%", math.floor(v[2] * 100), math.floor((0.15 + v[4] * 0.85) * 100)))
  screen.update()
end
