-- life
-- a Portamax norns script
--
-- conway's game of life on a
-- 16 x 8 board. a scanner sweeps
-- across it; living cells in its
-- column sound, low rows low.
--
-- E2 tempo   E3 generations per sweep
-- K2 reseed   K3 glider
-- (params: scale, root)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local W, H = 16, 8
local board = {}
local col = 0
local scale = {}
local gens = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), H)
end

local function reseed()
  for x = 1, W do
    board[x] = {}
    for y = 1, H do board[x][y] = math.random() < 0.3 end
  end
end

local function glider()
  local x, y = math.random(1, W), math.random(1, H)
  for _, p in ipairs({ { 1, 0 }, { 2, 1 }, { 0, 2 }, { 1, 2 }, { 2, 2 } }) do
    board[(x + p[1] - 1) % W + 1][(y + p[2] - 1) % H + 1] = true
  end
end

local function evolve()
  local nxt = {}
  local alive = 0
  for x = 1, W do
    nxt[x] = {}
    for y = 1, H do
      local n = 0
      for dx = -1, 1 do
        for dy = -1, 1 do
          if (dx ~= 0 or dy ~= 0) and board[(x + dx - 1) % W + 1][(y + dy - 1) % H + 1] then n = n + 1 end
        end
      end
      nxt[x][y] = (n == 3) or (board[x][y] and n == 2)
      if nxt[x][y] then alive = alive + 1 end
    end
  end
  board = nxt
  gens = gens + 1
  if alive < 4 then reseed() end
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("LIFE")
  params:add_number("per_sweep", "generations/sweep", 1, 4, 1)
  params:add_option("scale", "scale", names, 11)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 48, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:default()
  engine.release(0.4)
  engine.amp(0.14)
  engine.cutoff(2000)
  math.randomseed(os.time())
  build_scale()
  reseed()
  clock.run(function()
    while true do
      clock.sync(1 / 4)
      col = col % W + 1
      if col == 1 then for _ = 1, params:get("per_sweep") do evolve() end end
      local sung = 0
      for y = 1, H do
        if board[col][y] and sung < 3 then
          sung = sung + 1
          engine.pan(col / 8 - 1)
          engine.hz(MusicUtil.note_num_to_freq(scale[H - y + 1] + (sung - 1) * 12))
        end
      end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("clock_tempo", d)
  elseif n == 3 then params:delta("per_sweep", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then reseed() elseif n == 3 then glider() end
  redraw()
end

function redraw()
  screen.clear()
  for x = 1, W do
    for y = 1, H do
      if board[x][y] then
        screen.level(x == col and 15 or 6)
        screen.rect((x - 1) * 8 + 1, 10 + (y - 1) * 6, 6, 5)
        screen.fill()
      end
    end
  end
  screen.level(3)
  screen.rect((col - 1) * 8, 9, 8, 48)
  screen.stroke()
  screen.level(15)
  screen.move(0, 7)
  screen.text("life")
  screen.level(4)
  screen.move(127, 7)
  screen.text_right("gen " .. gens)
  screen.update()
end
