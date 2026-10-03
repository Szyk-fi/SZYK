-- coral
-- a Portamax norns script
--
-- a reef grows by diffusion-
-- limited aggregation: drifting
-- particles wander until they
-- touch the coral and stick.
-- each one that sticks plays a
-- note set by its height, so the
-- melody climbs as the reef does.
--
-- E2 current    E3 brightness
-- K2 new reef   K3 release a school
-- pads: a particle at that spot
-- (params: scale, root, walkers)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local W, H = 128, 52
local TOP = 11
local cells = {}
local walkers = {}
local scale = {}
local top_y = H
local count = 0
local last_note = 0
local t = 0
local sparks = {}

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 18)
end

local function idx(x, y) return y * W + x end
local function occupied(x, y)
  if x < 0 or x >= W or y < 0 or y >= H then return false end
  return cells[idx(x, y)] ~= nil
end

local function launch_y()
  return math.max(0, top_y - 8 - math.random(0, 4))
end

local function new_walker(x, y)
  return { x = x or math.random(0, W - 1), y = y or launch_y() }
end

local function new_reef()
  cells = {}
  count = 0
  top_y = H - 1
  -- a few polyps on the sea floor to grow from
  for i = 1, 5 do
    local x = math.floor(i * W / 6 + math.random(-6, 6))
    cells[idx(x, H - 1)] = 0
    count = count + 1
  end
  walkers = {}
  for _ = 1, params:get("walkers") do table.insert(walkers, new_walker()) end
end

local function stick(w)
  count = count + 1
  cells[idx(w.x, w.y)] = t
  local tip = w.y < top_y
  if tip then top_y = w.y end
  local height = H - 1 - w.y
  table.insert(sparks, { x = w.x, y = w.y + TOP, life = 6 })
  -- notes are spaced a little so a busy reef stays melodic
  if t - last_note > 0.09 then
    last_note = t
    local deg = util.clamp(1 + math.floor(height / H * (#scale - 1)), 1, #scale)
    engine.pan((w.x / W - 0.5) * 1.6)
    engine.amp(tip and 0.24 or 0.13)
    engine.pw(tip and 0.2 or 0.45)
    engine.release(tip and 1.6 or 0.6)
    engine.cutoff(params:get("bright") * (tip and 1.5 or 0.8))
    engine.hz(MusicUtil.note_num_to_freq(scale[deg]))
  end
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("CORAL")
  params:add_option("scale", "scale", names, 2)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 64, 45, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("current", "current", controlspec.new(0, 0.6, 'lin', 0, 0.2, ''))
  params:add_number("walkers", "walkers", 2, 24, 10)
  params:add_control("bright", "brightness", controlspec.new(300, 8000, 'exp', 0, 1800, 'hz'))
  params:default()
  engine.gain(1.2)
  math.randomseed(os.time())
  build_scale()
  new_reef()
  -- the first drifters start right beside the polyps
  local k = 0
  for i, _ in pairs(cells) do
    k = k + 1
    if walkers[k] then walkers[k].x = i % W + 1 walkers[k].y = H - 3 end
  end
  midi.connect().event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      table.insert(walkers, new_walker(((msg.note * 17) % W), launch_y()))
    end
  end
  local m = metro.init(step, 1 / 30)
  m:start()
end

function step()
  t = t + 1 / 30
  local drift = params:get("current")
  for i = #walkers, 1, -1 do
    local w = walkers[i]
    local stuck = false
    for _ = 1, 30 do
      -- a random walk with a downward settling bias
      local r = math.random()
      if r < 0.25 then w.x = w.x - 1
      elseif r < 0.5 then w.x = w.x + 1
      elseif r < 0.75 + drift * 0.4 then w.y = w.y + 1
      else w.y = w.y - 1 end
      w.x = w.x % W
      if w.y < 0 then w.y = 0 end
      if w.y > H - 1 then w.y = H - 1 end
      if not occupied(w.x, w.y) and (occupied(w.x - 1, w.y) or occupied(w.x + 1, w.y)
        or occupied(w.x, w.y + 1) or occupied(w.x, w.y - 1)) then
        stick(w)
        stuck = true
        break
      end
    end
    if stuck then
      if #walkers > params:get("walkers") then table.remove(walkers, i)
      else walkers[i] = new_walker() end
    end
  end
  -- when the reef reaches the surface it bleaches and starts over
  if top_y < 3 or count > 1600 then new_reef() end
  for i = #sparks, 1, -1 do
    sparks[i].life = sparks[i].life - 1
    if sparks[i].life <= 0 then table.remove(sparks, i) end
  end
  -- the reef can hold many pixels; 15 fps is plenty for growth
  if math.floor(t * 30 + 0.5) % 2 == 0 then redraw() end
end

function enc(n, d)
  if n == 2 then params:delta("current", d)
  elseif n == 3 then params:delta("bright", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    new_reef()
  elseif n == 3 then
    for _ = 1, 8 do table.insert(walkers, new_walker(nil, launch_y() + 4)) end
  end
end

function redraw()
  screen.clear()
  for i, born in pairs(cells) do
    local x = i % W
    local y = i // W
    -- newer growth is brighter
    screen.level(util.clamp(15 - math.floor((t - born) * 0.6), 4, 15))
    screen.pixel(x, y + TOP)
    screen.fill()
  end
  screen.level(3)
  for _, w in ipairs(walkers) do
    screen.pixel(w.x, w.y + TOP)
    screen.fill()
  end
  for _, s in ipairs(sparks) do
    screen.level(s.life * 2)
    screen.circle(s.x, s.y, 7 - s.life)
    screen.stroke()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text("coral")
  screen.level(4)
  screen.move(127, 8)
  screen.text_right(count .. " polyps")
  screen.update()
end
