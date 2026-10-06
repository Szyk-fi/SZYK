-- automata
-- a Portamax norns script
-- after ricochet by bongo (monome
-- community) and Otomata by
-- Batuhan Bozkurt: a generative
-- sequencer of little travellers.
--
-- press a key to place a cell going
-- up; press it again to turn it
-- (up, right, down, left, gone).
-- a cell sounds when it hits a wall
-- (its column or row is the note)
-- and turns back. cells that meet
-- on one key all turn right.
--
-- E1 tempo  E2 scale  E3 root
-- K2 clear  K3 scatter new cells
-- open the Grid app to play

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local g = grid.connect()
local cells = {}
local scale = {}
local hits = {}
local SCALES = { 12, 11, 5, 2, 8 }
local scale_i = 1
local root = 50

local DIRS = { { 0, -1 }, { 1, 0 }, { 0, 1 }, { -1, 0 } }

local function cols() return math.max(g.cols, 1) end
local function rows() return math.max(g.rows, 1) end

local function build()
  scale = MusicUtil.generate_scale_of_length(root, SCALES[scale_i], 64)
end

local function find(x, y)
  for i, c in ipairs(cells) do
    if c.x == x and c.y == y then return i end
  end
end

local function step()
  local sounded = 0
  for _, c in ipairs(cells) do
    local dx, dy = DIRS[c.d][1], DIRS[c.d][2]
    local nx, ny = c.x + dx, c.y + dy
    if nx < 1 or nx > cols() or ny < 1 or ny > rows() then
      c.d = (c.d + 1) % 4 + 1
      nx, ny = c.x - dx, c.y - dy
      -- vertical travellers sound their column, horizontal their row
      local degree = dy ~= 0 and c.x or (rows() - c.y + 1)
      if sounded < 4 then
        engine.hz(MusicUtil.note_num_to_freq(scale[util.clamp(degree, 1, #scale)]))
        sounded = sounded + 1
      end
      hits[#hits + 1] = { x = c.x, y = c.y, l = 15 }
    end
    c.x = util.clamp(nx, 1, cols())
    c.y = util.clamp(ny, 1, rows())
  end
  -- cells sharing a key turn clockwise
  local count = {}
  for _, c in ipairs(cells) do
    local k = c.x .. "," .. c.y
    count[k] = (count[k] or 0) + 1
  end
  for _, c in ipairs(cells) do
    if count[c.x .. "," .. c.y] > 1 then c.d = c.d % 4 + 1 end
  end
end

function grid_redraw()
  g:all(0)
  local keep = {}
  for _, h in ipairs(hits) do
    g:led(h.x, h.y, h.l)
    h.l = h.l - 4
    if h.l > 0 then keep[#keep + 1] = h end
  end
  hits = keep
  for _, c in ipairs(cells) do g:led(c.x, c.y, 8 + c.d) end
  g:refresh()
end

g.key = function(x, y, z)
  if z == 0 then return end
  local i = find(x, y)
  if not i then
    table.insert(cells, { x = x, y = y, d = 1 })
  elseif cells[i].d == 4 then
    table.remove(cells, i)
  else
    cells[i].d = cells[i].d + 1
  end
  grid_redraw()
  redraw()
end

local function scatter()
  for _ = 1, 5 do
    table.insert(cells, { x = math.random(1, cols()), y = math.random(1, rows()), d = math.random(1, 4) })
  end
end

function init()
  build()
  engine.amp(0.25)
  engine.release(1.2)
  engine.cutoff(2400)
  -- a few starting at the walls, so it sings straight away
  table.insert(cells, { x = 1, y = math.min(2, rows()), d = 4 })
  table.insert(cells, { x = cols(), y = math.max(rows() - 1, 1), d = 2 })
  table.insert(cells, { x = math.min(3, cols()), y = 1, d = 1 })
  scatter()
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
  elseif n == 2 then scale_i = util.clamp(scale_i + d, 1, #SCALES); build()
  elseif n == 3 then root = util.clamp(root + d, 30, 70); build() end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then cells = {} elseif n == 3 then scatter() end
  grid_redraw()
  redraw()
end

function redraw()
  screen.clear()
  local w, h = math.floor(124 / cols()), math.floor(40 / rows())
  local arrows = { "^", ">", "v", "<" }
  for _, c in ipairs(cells) do
    screen.level(10)
    screen.move(2 + (c.x - 1) * w, 12 + c.y * h)
    screen.text(arrows[c.d])
  end
  screen.level(15)
  screen.move(0, 7)
  screen.text("automata")
  screen.level(4)
  screen.move(128, 7)
  screen.text_right(MusicUtil.SCALES[SCALES[scale_i]].name)
  screen.move(0, 62)
  screen.text(#cells .. " cells  root " .. MusicUtil.note_num_to_name(root, true))
  screen.update()
end
