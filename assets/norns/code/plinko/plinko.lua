-- plinko
-- a Portamax norns script
-- after Plinkonome by JP (monome
-- community): the Price is Right
-- game as an instrument.
--
-- the top row drops a chip down that
-- column. every other key places or
-- removes a peg. a chip that hits a
-- peg sounds that peg's note and
-- bounces left or right; one that
-- lands sounds a low note.
-- chips drop by themselves too.
--
-- E1 tempo  E2 auto drop  E3 root
-- K2 clear pegs  K3 new pegs
-- open the Grid app to play

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local g = grid.connect()
local pegs = {}
local chips = {}
local lit = {}
local auto = 4
local tick = 0
local root = 48
local scale = {}

local function cols() return math.max(g.cols, 1) end
local function rows() return math.max(g.rows, 1) end

local function peg(x, y) return pegs[y] and pegs[y][x] end
local function set_peg(x, y, on)
  pegs[y] = pegs[y] or {}
  pegs[y][x] = on or nil
end

local function deg(i) return scale[util.clamp(i, 1, #scale)] end

local function note(n, amp)
  engine.amp(amp or 0.3)
  engine.hz(MusicUtil.note_num_to_freq(n))
end

local function new_pegs()
  pegs = {}
  for y = 3, rows() - 1, 2 do
    for x = 1, cols() do
      if (x + y) % 2 == 0 and math.random() < 0.6 then set_peg(x, y, true) end
    end
  end
end

local function drop(x)
  if #chips < 12 then
    table.insert(chips, { x = x, y = 1 })
    -- a soft click as it leaves the hand
    note(deg(x) + 24, 0.08)
  end
end

local function step()
  local keep = {}
  for _, c in ipairs(chips) do
    local ny = c.y + 1
    if ny > rows() then
      note(deg(c.x) - 12, 0.4)
      lit[#lit + 1] = { x = c.x, y = rows(), l = 15 }
    elseif peg(c.x, ny) then
      -- a peg: its note, then off to one side
      note(deg(rows() - ny + c.x % 3 + 1) + 12)
      lit[#lit + 1] = { x = c.x, y = ny, l = 15 }
      local side = math.random() < 0.5 and -1 or 1
      if c.x + side < 1 or c.x + side > cols() then side = -side end
      c.x = c.x + side
      keep[#keep + 1] = c
    else
      c.y = ny
      keep[#keep + 1] = c
    end
  end
  chips = keep
  tick = tick + 1
  if auto > 0 and tick % (auto * 4) == 0 then drop(math.random(1, cols())) end
end

function grid_redraw()
  g:all(0)
  for y, row in pairs(pegs) do
    for x in pairs(row) do g:led(x, y, 4) end
  end
  local keep = {}
  for _, l in ipairs(lit) do
    g:led(l.x, l.y, l.l)
    l.l = l.l - 3
    if l.l > 0 then keep[#keep + 1] = l end
  end
  lit = keep
  for _, c in ipairs(chips) do g:led(c.x, c.y, 15) end
  g:refresh()
end

g.key = function(x, y, z)
  if z == 0 then return end
  if y == 1 then drop(x) else set_peg(x, y, not peg(x, y)) end
  grid_redraw()
end

function init()
  scale = MusicUtil.generate_scale_of_length(root, 11, 64)
  engine.release(0.8)
  engine.cutoff(2000)
  new_pegs()
  drop(math.max(1, math.floor(cols() / 2)))
  clock.run(function()
    while true do
      clock.sync(1 / 4)
      step()
      grid_redraw()
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 1 then params:delta("clock_tempo", d)
  elseif n == 2 then auto = util.clamp(auto + d, 0, 16)
  elseif n == 3 then
    root = util.clamp(root + d, 30, 66)
    scale = MusicUtil.generate_scale_of_length(root, 11, 64)
  end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then pegs = {} elseif n == 3 then new_pegs() end
  grid_redraw()
end

function redraw()
  screen.clear()
  local w, h = math.floor(124 / cols()), math.floor(40 / rows())
  screen.level(4)
  for y, row in pairs(pegs) do
    for x in pairs(row) do
      screen.circle(4 + (x - 1) * w, 12 + (y - 1) * h, 1)
      screen.fill()
    end
  end
  screen.level(15)
  for _, c in ipairs(chips) do
    screen.circle(4 + (c.x - 1) * w, 12 + (c.y - 1) * h, 2)
    screen.fill()
  end
  screen.move(0, 7)
  screen.text("plinko")
  screen.level(4)
  screen.move(128, 7)
  screen.text_right(auto == 0 and "drops: yours" or ("drop every " .. auto))
  screen.move(0, 62)
  screen.text("root " .. MusicUtil.note_num_to_name(root, true))
  screen.update()
end
