-- isogrid
-- a Portamax norns script
-- after mabalhabla by stretta
-- (monome community): a tonal grid
-- with its own interval across and
-- up, so every chord shape plays the
-- same anywhere.
--
-- press keys to play. lit keys are
-- the root's note in every octave;
-- keys sharing a held note glow.
-- K3 latches: then a key adds its
-- note to a chord (or takes it out)
-- and the chord plays as an arpeggio.
--
-- E1 tempo  E2 across  E3 up
-- K2 clear chord  K3 latch on/off
-- open the Grid app to play

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local g = grid.connect()
local across = 2
local up = 5
local root = 36
local held = {}
local chord = {}
local latch = true
local arp_i = 0
local playing = nil

local function cols() return math.max(g.cols, 1) end
local function rows() return math.max(g.rows, 1) end

-- the bottom-left key is the root
local function note_at(x, y) return root + (x - 1) * across + (rows() - y) * up end

local function play(n)
  engine.hz(MusicUtil.note_num_to_freq(util.clamp(n, 0, 127)))
  playing = n
end

local function in_chord(n)
  for i, c in ipairs(chord) do
    if c == n then return i end
  end
end

function grid_redraw()
  g:all(0)
  local sounding = {}
  for _, n in pairs(held) do sounding[n % 12] = true end
  for _, n in ipairs(chord) do sounding[n % 12] = true end
  for y = 1, rows() do
    for x = 1, cols() do
      local n = note_at(x, y)
      local l = 0
      if n % 12 == root % 12 then l = 3 end
      if sounding[n % 12] then l = 6 end
      if in_chord(n) then l = 10 end
      if n == playing then l = 15 end
      g:led(x, y, l)
    end
  end
  for k in pairs(held) do
    local x, y = k:match("(%d+),(%d+)")
    g:led(tonumber(x), tonumber(y), 15)
  end
  g:refresh()
end

g.key = function(x, y, z)
  local k = x .. "," .. y
  local n = note_at(x, y)
  if z == 1 then
    held[k] = n
    if latch then
      local i = in_chord(n)
      if i then table.remove(chord, i) else table.insert(chord, n); table.sort(chord) end
    end
    play(n)
  else
    held[k] = nil
  end
  grid_redraw()
  redraw()
end

function init()
  engine.amp(0.3)
  engine.release(0.9)
  engine.cutoff(2200)
  -- a chord to start from: root, a fifth, a tenth
  chord = { root + 12, root + 19, root + 28 }
  clock.run(function()
    while true do
      clock.sync(1 / 4)
      if latch and #chord > 0 then
        arp_i = arp_i % #chord + 1
        play(chord[arp_i])
        grid_redraw()
        redraw()
      end
    end
  end)
end

function enc(n, d)
  if n == 1 then params:delta("clock_tempo", d)
  elseif n == 2 then across = util.clamp(across + d, 1, 12)
  elseif n == 3 then up = util.clamp(up + d, 1, 12) end
  grid_redraw()
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then chord = {} elseif n == 3 then latch = not latch end
  grid_redraw()
  redraw()
end

function redraw()
  screen.clear()
  screen.level(15)
  screen.move(0, 7)
  screen.text("isogrid")
  screen.level(4)
  screen.move(128, 7)
  screen.text_right(latch and "latched" or "play")
  screen.level(10)
  screen.move(0, 26)
  screen.text("across +" .. across .. "   up +" .. up)
  local names = {}
  for _, c in ipairs(chord) do names[#names + 1] = MusicUtil.note_num_to_name(c, true) end
  screen.level(6)
  screen.move(0, 42)
  screen.text(#names > 0 and table.concat(names, " ") or "no chord")
  if playing then
    screen.level(15)
    screen.move(128, 62)
    screen.text_right(MusicUtil.note_num_to_name(playing, true))
  end
  screen.update()
end
