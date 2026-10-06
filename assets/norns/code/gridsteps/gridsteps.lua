-- gridsteps
-- a Portamax norns script
--
-- a step sequencer you play on the
-- grid: a column per step, a row per
-- note of the scale (top is highest).
-- press a key to set that step's
-- note; press it again to rest.
-- fits whatever size the grid is.
--
-- E1 tempo   E2 length   E3 octave
-- K2 clear   K3 random
-- open the Grid app to play the keys

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local g = grid.connect()
local notes = {}  -- notes[step] = row (1 = top) or 0 for a rest
local length = 16
local playhead = 0
local octave = 0
local scale = {}

local function rows() return math.max(g.rows, 1) end
local function cols() return math.max(g.cols, 1) end

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 64)
end

local function note_of(row)
  -- the bottom row is the root
  local degree = rows() - row + 1
  return scale[degree] + 12 * octave
end

local function randomize()
  for i = 1, 128 do
    notes[i] = (math.random() < 0.7) and math.random(1, rows()) or 0
  end
end

function grid_redraw()
  g:all(0)
  for x = 1, math.min(cols(), length) do
    -- the length shows as a dim floor along the bottom row
    g:led(x, rows(), 2)
    if notes[x] and notes[x] > 0 then
      g:led(x, notes[x], x == playhead and 15 or 8)
    elseif x == playhead then
      g:led(x, rows(), 6)
    end
  end
  g:refresh()
end

g.key = function(x, y, z)
  if z == 0 or x > 128 then return end
  if notes[x] == y then notes[x] = 0 else notes[x] = y end
  grid_redraw()
  redraw()
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("GRIDSTEPS")
  params:add_option("scale", "scale", names, 5)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 24, 60, 48, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("release", "release", controlspec.new(0.05, 2, 'exp', 0, 0.4, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:default()
  engine.amp(0.3)
  build_scale()
  length = math.min(16, cols())
  -- a tune to start from, so it plays before anything is pressed
  local tune = { 8, 0, 6, 0, 5, 6, 0, 4, 8, 0, 6, 0, 3, 4, 5, 0 }
  for i = 1, 128 do
    local r = tune[(i - 1) % 16 + 1]
    notes[i] = (r > 0) and math.min(r, rows()) or 0
  end
  clock.run(function()
    while true do
      clock.sync(1 / 4)
      playhead = playhead % length + 1
      local r = notes[playhead]
      if r and r > 0 then engine.hz(MusicUtil.note_num_to_freq(note_of(r))) end
      grid_redraw()
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 1 then params:delta("clock_tempo", d)
  elseif n == 2 then length = util.clamp(length + d, 1, math.min(cols(), 128))
  elseif n == 3 then octave = util.clamp(octave + d, -2, 2) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    for i = 1, 128 do notes[i] = 0 end
  elseif n == 3 then
    randomize()
  end
  grid_redraw()
  redraw()
end

function redraw()
  screen.clear()
  -- a small picture of the grid: as many steps and rows as fit
  local c, r = math.min(cols(), 32), math.min(rows(), 12)
  local w, h = math.floor(124 / c), math.floor(40 / r)
  for x = 1, math.min(c, length) do
    local row = notes[x] or 0
    if row > 0 and row <= r then
      screen.level(x == playhead and 15 or 6)
      screen.rect(2 + (x - 1) * w, 12 + (row - 1) * h, math.max(w - 1, 1), math.max(h - 1, 1))
      screen.fill()
    end
  end
  screen.level(15)
  screen.move(0, 7)
  screen.text("gridsteps")
  screen.level(4)
  screen.move(128, 7)
  screen.text_right(cols() .. "x" .. rows() .. "  len " .. length)
  screen.move(0, 62)
  screen.text("oct " .. octave .. "  " .. g.name)
  screen.update()
end
