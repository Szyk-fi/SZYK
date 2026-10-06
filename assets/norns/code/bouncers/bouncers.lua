-- bouncers
-- a Portamax norns script
-- after Rebound by josh g. (monome
-- community, 2008): notes bounce
-- across the grid.
--
-- press a key: a ball starts there.
-- press a ball: it turns a quarter.
-- a ball plays when it hits a wall:
-- the wall's row or column is the
-- note. now and then one swerves.
--
-- E1 tempo  E2 swerve  E3 octave
-- K2 clear  K3 three new balls
-- open the Grid app to play

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local g = grid.connect()
local balls = {}
local swerve = 0.05
local octave = 0
local scale = {}
local flash = {}

local DIRS = { { 1, 0 }, { 0, 1 }, { -1, 0 }, { 0, -1 } }

local function cols() return math.max(g.cols, 1) end
local function rows() return math.max(g.rows, 1) end

local function add(x, y, d)
  if #balls >= 16 then table.remove(balls, 1) end
  table.insert(balls, { x = x, y = y, d = d or 1 })
end

local function play(pos, span)
  -- the wall position, low to high, on the scale
  local degree = util.clamp(pos, 1, math.min(span, #scale))
  local note = scale[degree] + 12 * octave
  engine.hz(MusicUtil.note_num_to_freq(note))
end

local function step()
  for _, b in ipairs(balls) do
    if math.random() < swerve then
      b.d = (b.d + (math.random() < 0.5 and 0 or 2)) % 4 + 1
    end
    local dx, dy = DIRS[b.d][1], DIRS[b.d][2]
    local nx, ny = b.x + dx, b.y + dy
    if nx < 1 or nx > cols() or ny < 1 or ny > rows() then
      -- off the edge: bounce back and sound
      b.d = (b.d + 1) % 4 + 1
      if dx ~= 0 then play(rows() - b.y + 1, rows()) else play(b.x, cols()) end
      flash[b.x .. "," .. b.y] = 15
      nx, ny = b.x - dx, b.y - dy
    end
    b.x = util.clamp(nx, 1, cols())
    b.y = util.clamp(ny, 1, rows())
  end
end

function grid_redraw()
  g:all(0)
  for k, l in pairs(flash) do
    local x, y = k:match("(%d+),(%d+)")
    g:led(tonumber(x), tonumber(y), l)
    flash[k] = l > 3 and l - 3 or nil
  end
  for _, b in ipairs(balls) do g:led(b.x, b.y, 12) end
  g:refresh()
end

g.key = function(x, y, z)
  if z == 0 then return end
  for _, b in ipairs(balls) do
    if b.x == x and b.y == y then
      b.d = b.d % 4 + 1
      grid_redraw()
      return
    end
  end
  add(x, y, math.random(1, 4))
  grid_redraw()
end

local function seed()
  for _ = 1, 3 do add(math.random(1, cols()), math.random(1, rows()), math.random(1, 4)) end
end

function init()
  scale = MusicUtil.generate_scale_of_length(45, 12, 64)
  engine.amp(0.3)
  engine.release(0.5)
  engine.cutoff(1800)
  -- one about to hit the left wall, so it starts at once
  add(1, math.max(1, rows() - 1), 3)
  seed()
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
  elseif n == 2 then swerve = util.clamp(swerve + d / 100, 0, 0.5)
  elseif n == 3 then octave = util.clamp(octave + d, -2, 2) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then balls = {} elseif n == 3 then seed() end
  grid_redraw()
  redraw()
end

function redraw()
  screen.clear()
  local w, h = math.floor(124 / cols()), math.floor(40 / rows())
  screen.level(2)
  screen.rect(1, 11, w * cols() + 2, h * rows() + 2)
  screen.stroke()
  screen.level(15)
  for _, b in ipairs(balls) do
    screen.rect(2 + (b.x - 1) * w, 12 + (b.y - 1) * h, math.max(w - 1, 1), math.max(h - 1, 1))
    screen.fill()
  end
  screen.move(0, 7)
  screen.text("bouncers")
  screen.level(4)
  screen.move(128, 7)
  screen.text_right(#balls .. " balls")
  screen.move(0, 62)
  screen.text("swerve " .. math.floor(swerve * 100) .. "%  oct " .. octave)
  screen.update()
end
