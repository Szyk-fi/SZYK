-- sandpile
-- a Portamax norns script
--
-- grains fall onto a table. any
-- cell holding four grains topples,
-- passing one to each neighbour,
-- which may topple in turn. each
-- avalanche plays a run of notes,
-- as long as the avalanche is big.
--
-- E2 drop rate   E3 brightness
-- K2 pour a handful   K3 pause
-- pads: drop a grain in that column
-- (params: scale, root, max burst)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local W, H, C = 18, 7, 6
local grid = {}
local hot = {}
local scale = {}
local paused = false
local last_size = 0
local ox, oy = 10, 12

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 16)
end

local function burst(size, x)
  local n = math.min(size, params:get("burst"))
  clock.run(function()
    for i = 1, n do
      engine.pan(util.linlin(1, W, -0.8, 0.8, x))
      engine.pw(0.2 + 0.05 * (i % 4))
      engine.release(0.25 + 0.08 * n)
      -- a falling run: starts higher the bigger the slide
      engine.hz(MusicUtil.note_num_to_freq(scale[util.clamp(n - i + 1 + x // 6, 1, #scale)]))
      clock.sleep(0.07)
    end
  end)
end

local function drop(x, y)
  grid[y][x] = grid[y][x] + 1
  local size = 0
  local stack = { { x, y } }
  while #stack > 0 do
    local c = table.remove(stack)
    local cx, cy = c[1], c[2]
    if grid[cy][cx] >= 4 then
      grid[cy][cx] = grid[cy][cx] - 4
      hot[cy][cx] = 15
      size = size + 1
      for _, d in ipairs({ { 1, 0 }, { -1, 0 }, { 0, 1 }, { 0, -1 } }) do
        local nx, ny = cx + d[1], cy + d[2]
        if nx >= 1 and nx <= W and ny >= 1 and ny <= H then
          grid[ny][nx] = grid[ny][nx] + 1
          if grid[ny][nx] >= 4 then table.insert(stack, { nx, ny }) end
        end
      end
      if grid[cy][cx] >= 4 then table.insert(stack, { cx, cy }) end
    end
  end
  if size > 0 then
    last_size = size
    burst(size, x)
  else
    -- a lone grain: a faint tick, pitched by its row
    engine.pw(0.6)
    engine.release(0.12)
    engine.pan(util.linlin(1, W, -0.8, 0.8, x))
    engine.hz(MusicUtil.note_num_to_freq(scale[H + 1 - y] + 24))
  end
end

local function random_drop()
  -- biased toward the middle, like pouring from above
  local x = util.clamp(math.floor(W / 2 + (math.random() + math.random() - 1) * W / 2) + 1, 1, W)
  drop(x, math.random(1, H))
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("SANDPILE")
  params:add_option("scale", "scale", names, 12)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 52, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("rate", "drop rate", controlspec.new(0.5, 16, 'exp', 0, 4, '/s'))
  params:add_number("burst", "max burst", 2, 16, 10)
  params:add_control("bright", "brightness", controlspec.new(300, 6000, 'exp', 0, 2000, 'hz'))
  params:set_action("bright", function(x) engine.cutoff(x) end)
  params:default()
  engine.amp(0.22)
  math.randomseed(os.time())
  build_scale()
  for y = 1, H do
    grid[y], hot[y] = {}, {}
    for x = 1, W do grid[y][x] = math.random(1, 3) hot[y][x] = 0 end
  end
  grid[4][9] = 3
  drop(9, 4)
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then drop((msg.note % W) + 1, math.random(1, H)) end
  end
  clock.run(function()
    while true do
      clock.sleep(1 / params:get("rate"))
      if not paused then random_drop() end
    end
  end)
  metro.init(function()
    for y = 1, H do for x = 1, W do hot[y][x] = math.max(0, hot[y][x] - 1) end end
    redraw()
  end, 1 / 20):start()
end

function enc(n, d)
  if n == 2 then params:delta("rate", d)
  elseif n == 3 then params:delta("bright", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then for _ = 1, 6 do drop(math.random(7, 12), math.random(3, 5)) end
  elseif n == 3 then paused = not paused end
end

function redraw()
  screen.clear()
  for y = 1, H do
    for x = 1, W do
      local g = grid[y][x]
      local px, py = ox + (x - 1) * C, oy + (y - 1) * C
      if hot[y][x] > 0 then
        screen.level(hot[y][x])
        screen.rect(px, py, C - 1, C - 1)
        screen.fill()
      elseif g > 0 then
        screen.level(g * 3)
        screen.rect(px + (3 - g), py + (3 - g), g + 1, g + 1)
        screen.fill()
      end
    end
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text(paused and "sandpile (paused)" or "sandpile")
  screen.level(4)
  screen.move(0, 62)
  screen.text(params:string("rate"))
  screen.move(127, 62)
  screen.text_right("last avalanche " .. last_size)
  screen.update()
end
