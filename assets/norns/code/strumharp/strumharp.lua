-- strumharp
-- a Portamax norns script
-- after autoharp by Meng Qi (monome
-- community): strummers with speed
-- and resistance.
--
-- the top row picks the chord (I to
-- vii and on, up the scale). every
-- other row is a strummer: press a
-- key and a pick flies off across
-- the strings (columns), plucking as
-- it goes - the further right you
-- press, the harder you threw it -
-- and slows down until it stops.
-- lower rows are lower octaves.
--
-- E1 tempo  E2 resistance  E3 root
-- K2 chords change by themselves
-- K3 strum every row
-- open the Grid app to play

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local g = grid.connect()
local root = 48
local slot = 1
local auto = true
local resistance = 0.06
local picks = {}
local beat = 0
local PROGRESSION = { 1, 6, 4, 5 }

local function cols() return math.max(g.cols, 1) end
local function rows() return math.max(g.rows, 1) end

-- the chord on scale degree `d` of the major scale, as semitones
local function chord_of(d)
  local major = { 0, 2, 4, 5, 7, 9, 11 }
  local out = {}
  for k = 0, 2 do
    local i = d - 1 + k * 2
    out[#out + 1] = major[i % 7 + 1] + 12 * math.floor(i / 7)
  end
  return out
end

local function string_note(col, row)
  local c = chord_of(slot)
  local i = col - 1
  local octave = math.floor(i / #c) + math.floor((rows() - row) / 2) - 1
  return root + c[i % #c + 1] + 12 * octave
end

local function throw(row, x)
  -- speed in strings per tick
  picks[row] = { pos = 1, vel = 0.15 + 0.85 * x / cols(), last = 0 }
end

local function step()
  for row, p in pairs(picks) do
    p.pos = p.pos + p.vel
    p.vel = p.vel - resistance * p.vel
    local s = math.floor(p.pos)
    if s ~= p.last and s <= cols() then
      p.last = s
      engine.amp(0.12 + p.vel * 0.3)
      engine.hz(MusicUtil.note_num_to_freq(util.clamp(string_note(s, row), 0, 120)))
    end
    if p.vel < 0.03 or s > cols() then picks[row] = nil end
  end
end

function grid_redraw()
  g:all(0)
  for x = 1, math.min(cols(), 7) do g:led(x, 1, x == slot and 15 or 4) end
  for row = 2, rows() do
    for x = 1, cols() do
      -- the strings: chord roots glow a little
      if (x - 1) % 3 == 0 then g:led(x, row, 2) end
    end
    local p = picks[row]
    if p and math.floor(p.pos) <= cols() then g:led(math.floor(p.pos), row, 15) end
  end
  g:refresh()
end

g.key = function(x, y, z)
  if z == 0 then return end
  if y == 1 then
    if x <= 7 then slot = x end
  else
    throw(y, x)
  end
  grid_redraw()
  redraw()
end

local function strum_all()
  for row = 2, rows() do throw(row, cols() * (0.4 + 0.4 * math.random())) end
end

function init()
  engine.release(1.6)
  engine.cutoff(2600)
  throw(math.min(3, rows()), cols())
  clock.run(function()
    while true do
      clock.sync(1 / 16)
      step()
      grid_redraw()
    end
  end)
  clock.run(function()
    while true do
      clock.sync(2)
      beat = beat + 1
      if auto then
        slot = PROGRESSION[beat % #PROGRESSION + 1]
        throw(2 + beat % math.max(rows() - 1, 1), cols() * 0.7)
      end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 1 then params:delta("clock_tempo", d)
  elseif n == 2 then resistance = util.clamp(resistance + d / 200, 0.005, 0.3)
  elseif n == 3 then root = util.clamp(root + d, 36, 60) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then auto = not auto elseif n == 3 then strum_all() end
  redraw()
end

function redraw()
  screen.clear()
  local names = { "I", "ii", "iii", "IV", "V", "vi", "vii" }
  screen.level(15)
  screen.move(0, 7)
  screen.text("strumharp")
  screen.level(4)
  screen.move(128, 7)
  screen.text_right(auto and "auto chords" or "your chords")
  screen.font_size(16)
  screen.level(15)
  screen.move(0, 34)
  screen.text(MusicUtil.note_num_to_name(root, false) .. " " .. names[slot])
  screen.font_size(8)
  screen.level(6)
  screen.move(0, 62)
  screen.text("resistance " .. math.floor(resistance * 100))
  screen.update()
end
