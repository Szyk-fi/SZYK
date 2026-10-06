-- blocks
-- a Portamax norns script
-- after Tetris for monome by robb
-- (monome community): falling
-- blocks on the grid, with notes.
--
-- a wide grid is turned on its side
-- so the well is tall. press left or
-- right of the falling piece to move
-- it, on it to turn it, under it to
-- drop it. landing pieces play their
-- column; full lines play a chord.
-- left alone it plays itself.
--
-- E1 speed  E2 move  E3 turn
-- K2 drop  K3 new game
-- open the Grid app to play

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local g = grid.connect()
local W, H = 8, 16
local well = {}
local piece = nil
local lines = 0
local speed = 4
local idle = 0
local scale = {}

local SHAPES = {
  { { 0, 0 }, { 1, 0 }, { -1, 0 }, { 2, 0 } },  -- I
  { { 0, 0 }, { 1, 0 }, { 0, 1 }, { 1, 1 } },   -- O
  { { 0, 0 }, { -1, 0 }, { 1, 0 }, { 0, 1 } },  -- T
  { { 0, 0 }, { -1, 0 }, { 0, 1 }, { 1, 1 } },  -- S
  { { 0, 0 }, { 1, 0 }, { 0, 1 }, { -1, 1 } },  -- Z
  { { 0, 0 }, { -1, 0 }, { 1, 0 }, { 1, 1 } },  -- J
  { { 0, 0 }, { -1, 0 }, { 1, 0 }, { -1, 1 } }, -- L
}

local function cells(p)
  local out = {}
  for _, c in ipairs(p.shape) do
    local x, y = c[1], c[2]
    for _ = 1, p.rot do x, y = -y, x end
    out[#out + 1] = { p.x + x, p.y + y }
  end
  return out
end

local function fits(p)
  for _, c in ipairs(cells(p)) do
    local x, y = c[1], c[2]
    if x < 1 or x > W or y > H then return false end
    if y >= 1 and well[y][x] then return false end
  end
  return true
end

local function new_well()
  well = {}
  for y = 1, H do well[y] = {} end
  lines = 0
end

local function spawn()
  local kind = math.random(1, #SHAPES)
  piece = { shape = SHAPES[kind], x = math.floor(W / 2), y = 0, rot = 0 }
  -- each kind of piece has its own quiet note as it appears
  engine.amp(0.12)
  engine.hz(MusicUtil.note_num_to_freq(scale[kind + 7]))
  if not fits(piece) then
    -- game over: down the scale, then again
    for k = 1, 4 do
      engine.hz(MusicUtil.note_num_to_freq(scale[9 - k * 2] or 48))
    end
    new_well()
  end
end

local function try(dx, dy, drot)
  local p = { shape = piece.shape, x = piece.x + dx, y = piece.y + dy, rot = (piece.rot + drot) % 4 }
  if fits(p) then piece = p return true end
  return false
end

local function land()
  local cs = cells(piece)
  for _, c in ipairs(cs) do
    if c[2] >= 1 then well[c[2]][c[1]] = 8 + math.random(0, 4) end
  end
  engine.amp(0.3)
  engine.hz(MusicUtil.note_num_to_freq(scale[util.clamp(piece.x, 1, #scale)]))
  local cleared = 0
  local y = H
  while y >= 1 do
    local full = true
    for x = 1, W do if not well[y][x] then full = false end end
    if full then
      table.remove(well, y)
      table.insert(well, 1, {})
      cleared = cleared + 1
    else
      y = y - 1
    end
  end
  if cleared > 0 then
    lines = lines + cleared
    engine.amp(0.4)
    for k = 0, cleared + 1 do engine.hz(MusicUtil.note_num_to_freq(scale[1 + k * 2] + 12)) end
  end
  spawn()
end

local function fall()
  if not try(0, 1, 0) then land() end
end

local function drop()
  while try(0, 1, 0) do end
  land()
end

local function fit_grid()
  -- keep the well tall: turn a wide grid on its side
  if g.cols > g.rows then g:rotation(1) else g:rotation(0) end
  W, H = math.max(g.cols, 4), math.max(g.rows, 4)
  new_well()
  spawn()
end

function grid_redraw()
  g:all(0)
  for y = 1, H do
    for x = 1, W do
      if well[y][x] then g:led(x, y, well[y][x]) end
    end
  end
  if piece then
    for _, c in ipairs(cells(piece)) do
      if c[2] >= 1 then g:led(c[1], c[2], 15) end
    end
  end
  g:refresh()
end

g.key = function(x, y, z)
  if z == 0 or not piece then return end
  idle = 0
  local lo, hi, top, bottom = 99, -99, 99, -99
  for _, c in ipairs(cells(piece)) do
    lo, hi = math.min(lo, c[1]), math.max(hi, c[1])
    top, bottom = math.min(top, c[2]), math.max(bottom, c[2])
  end
  if y > bottom + 1 and x >= lo and x <= hi then drop()
  elseif x < lo then try(-1, 0, 0)
  elseif x > hi then try(1, 0, 0)
  else try(0, 0, 1) end
  grid_redraw()
end

-- nobody playing: wander about a bit
local function autopilot()
  local r = math.random()
  if r < 0.3 then try(-1, 0, 0) elseif r < 0.6 then try(1, 0, 0) elseif r < 0.75 then try(0, 0, 1) end
end

_px_grid_resize_old = _px_grid_resize
function _px_grid_resize(c, r)
  _px_grid_resize_old(c, r)
  fit_grid()
end

function init()
  scale = MusicUtil.generate_scale_of_length(48, 11, 64)
  engine.release(0.5)
  engine.cutoff(2500)
  fit_grid()
  clock.run(function()
    local t = 0
    while true do
      clock.sync(1 / 8)
      t = t + 1
      idle = idle + 1
      if idle > 80 then autopilot() end
      if t % math.max(9 - speed, 1) == 0 then fall() end
      grid_redraw()
      redraw()
    end
  end)
end

function enc(n, d)
  idle = 0
  if n == 1 then speed = util.clamp(speed + d, 1, 8)
  elseif n == 2 then try(d > 0 and 1 or -1, 0, 0)
  elseif n == 3 then try(0, 0, d > 0 and 1 or 3) end
  grid_redraw()
  redraw()
end

function key(n, z)
  if z == 0 then return end
  idle = 0
  if n == 2 then drop() elseif n == 3 then new_well(); spawn() end
  grid_redraw()
end

function redraw()
  screen.clear()
  -- the well, sideways across the screen
  local s = math.max(1, math.floor(math.min(110 / H, 40 / W)))
  screen.level(2)
  screen.rect(2, 12, H * s + 1, W * s + 1)
  screen.stroke()
  for y = 1, H do
    for x = 1, W do
      if well[y][x] then
        screen.level(6)
        screen.rect(3 + (y - 1) * s, 13 + (x - 1) * s, s, s)
        screen.fill()
      end
    end
  end
  if piece then
    screen.level(15)
    for _, c in ipairs(cells(piece)) do
      if c[2] >= 1 then
        screen.rect(3 + (c[2] - 1) * s, 13 + (c[1] - 1) * s, s, s)
        screen.fill()
      end
    end
  end
  screen.level(15)
  screen.move(0, 7)
  screen.text("blocks")
  screen.level(4)
  screen.move(128, 7)
  screen.text_right(lines .. " lines")
  screen.move(0, 62)
  screen.text("speed " .. speed .. (idle > 80 and "  (playing itself)" or ""))
  screen.update()
end
