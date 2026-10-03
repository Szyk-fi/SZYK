-- snake
-- a Portamax norns script
--
-- a snake steers itself toward
-- the food. each bite plays the
-- next note of the scale, and the
-- longer it grows the higher it
-- sings. a soft pulse keeps time.
--
-- E2 speed   E3 brightness
-- K2 new snake   K3 pause
-- pads: drop food at a column
-- (params: scale, root, foods)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local W, H, C, X0, Y0 = 30, 10, 4, 4, 12
local body = {}
local dir = { 1, 0 }
local food = {}
local scale = {}
local bites = 0
local steps = 0
local paused = false
local glow = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 22)
end

local function occupied(x, y)
  for _, s in ipairs(body) do if s[1] == x and s[2] == y then return true end end
  return false
end

local function add_food(x)
  for _ = 1, 30 do
    local fx, fy = x or math.random(W), math.random(H)
    if not occupied(fx, fy) then food[#food + 1] = { fx, fy } return end
  end
end

local function new_snake()
  body = { { 8, 5 }, { 7, 5 }, { 6, 5 } }
  dir = { 1, 0 }
  food = { { 11, 5 } }
  while #food < params:get("foods") do add_food() end
  bites = 0
  engine.amp(0.2)
  engine.hz(MusicUtil.note_num_to_freq(scale[1]))
end

local function wrap(x, y) return (x - 1) % W + 1, (y - 1) % H + 1 end

local function choose_dir()
  local h = body[1]
  local target, best = food[1], math.huge
  for _, f in ipairs(food) do
    local d = math.abs(f[1] - h[1]) + math.abs(f[2] - h[2])
    if d < best then target, best = f, d end
  end
  local options = { dir, { dir[2], -dir[1] }, { -dir[2], dir[1] } }
  local pick, score = nil, math.huge
  for _, o in ipairs(options) do
    local nx, ny = wrap(h[1] + o[1], h[2] + o[2])
    if not occupied(nx, ny) then
      local s = math.abs(target[1] - nx) + math.abs(target[2] - ny) + math.random() * 0.5
      if s < score then pick, score = o, s end
    end
  end
  return pick
end

local function tick()
  steps = steps + 1
  local d = choose_dir()
  if not d then
    -- boxed in: a low chord, then start again
    for i = 3, 1, -1 do engine.hz(MusicUtil.note_num_to_freq(scale[i])) end
    new_snake()
    return
  end
  dir = d
  local nx, ny = wrap(body[1][1] + dir[1], body[1][2] + dir[2])
  table.insert(body, 1, { nx, ny })
  local ate = false
  for i, f in ipairs(food) do
    if f[1] == nx and f[2] == ny then table.remove(food, i) ate = true break end
  end
  if ate then
    bites = bites + 1
    local lift = math.min(#body // 5, 10)
    engine.amp(0.3)
    engine.release(1.3)
    engine.pan((nx - W / 2) / W)
    engine.hz(MusicUtil.note_num_to_freq(scale[(bites - 1) % 7 + 1 + lift]))
    glow = 10
    add_food()
    if #body > 60 then new_snake() end
  else
    table.remove(body)
  end
  if steps % 4 == 0 then
    engine.amp(0.12)
    engine.release(0.6)
    engine.pan(0)
    engine.hz(MusicUtil.note_num_to_freq(scale[1] - 12))
  end
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("SNAKE")
  params:add_option("scale", "scale", names, 11)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 53, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_number("speed", "steps per beat", 1, 8, 4)
  params:add_number("foods", "foods", 1, 6, 3)
  params:add_control("bright", "brightness", controlspec.new(300, 8000, 'exp', 0, 2200, 'hz'))
  params:set_action("bright", function(x) engine.cutoff(x) end)
  params:default()
  math.randomseed(os.time())
  build_scale()
  new_snake()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then add_food(util.clamp((msg.note - 59) * 3, 1, W)) end
  end
  clock.run(function()
    while true do
      clock.sync(1 / params:get("speed"))
      if not paused then tick() end
      glow = math.max(0, glow - 1)
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("speed", d)
  elseif n == 3 then params:delta("bright", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_snake() elseif n == 3 then paused = not paused end
end

function redraw()
  screen.clear()
  screen.level(2)
  screen.rect(X0 - 1, Y0 - 1, W * C + 1, H * C + 1)
  screen.stroke()
  for _, f in ipairs(food) do
    screen.level(10)
    screen.rect(X0 + (f[1] - 1) * C + 1, Y0 + (f[2] - 1) * C + 1, 2, 2)
    screen.fill()
  end
  for i, s in ipairs(body) do
    screen.level(i == 1 and 15 or math.max(4, 12 - i // 3))
    screen.rect(X0 + (s[1] - 1) * C, Y0 + (s[2] - 1) * C, C - 1, C - 1)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text(paused and "snake (paused)" or "snake")
  screen.level(glow > 0 and 15 or 4)
  screen.move(127, 8)
  screen.text_right("length " .. #body)
  screen.level(4)
  screen.move(0, 62)
  screen.text("bites " .. bites)
  screen.move(127, 62)
  screen.text_right(MusicUtil.note_num_to_name(params:get("root"), true))
  screen.update()
end
