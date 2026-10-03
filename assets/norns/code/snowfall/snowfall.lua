-- snowfall
-- a Portamax norns script
--
-- flakes drift down on a slow
-- wind and settle into drifts.
-- a flake that lands on a drift's
-- crest rings a small bell; the
-- higher the drift, the higher
-- the bell. steep drifts slump.
--
-- E2 snowfall   E3 wind
-- K2 gust       K3 thaw
-- pads: a flurry over that bell
-- (params: scale, root, bells)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local W = 128
local GROUND = 63
local REPOSE = 2
local ground = {}
local flakes = {}
local glints = {}
local scale = {}
local t = 0
local gust = 0
local last_bell = -1

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 16)
end

local function new_ground()
  for x = 1, W do
    ground[x] = 3 + math.floor(2 + 2 * math.sin(x / 9) + math.random() * 2)
  end
end

local function spawn(x, y)
  table.insert(flakes, {
    x = x or math.random() * W,
    y = y or (8 + math.random() * 4),
    vy = 0.25 + math.random() * 0.35,
    ph = math.random() * 6.28,
    big = math.random() < 0.2,
  })
end

local function bell(x, h)
  -- sparse by design: never two bells closer than a short breath
  if t - last_bell < params:get("space") then return end
  last_bell = t
  local deg = util.clamp(math.floor(util.linlin(3, 40, 1, #scale, h)) + 1, 1, #scale)
  engine.pan((x / W - 0.5) * 1.6)
  engine.amp(0.18 + math.random() * 0.08)
  engine.pw(0.08)
  engine.release(3.2)
  engine.cutoff(params:get("bells"))
  engine.hz(MusicUtil.note_num_to_freq(scale[deg]))
  table.insert(glints, { x = x, y = GROUND - h, r = 1 })
end

-- grains slump off any slope steeper than the angle of repose
local function settle(x)
  for _ = 1, 6 do
    local moved = false
    for _, d in ipairs({ -1, 1 }) do
      local n = x + d
      if n >= 1 and n <= W and ground[x] - ground[n] > REPOSE then
        ground[x] = ground[x] - 1
        ground[n] = ground[n] + 1
        x = n
        moved = true
        break
      end
    end
    if not moved then return end
  end
end

local function land(f)
  local x = util.clamp(math.floor(f.x) + 1, 1, W)
  ground[x] = ground[x] + 1
  local l = ground[math.max(1, x - 1)]
  local r = ground[math.min(W, x + 1)]
  local h = ground[x]
  -- a crest: at least as high as both neighbours and above the local mean
  if h >= l and h >= r and h > (l + r) / 2 then
    bell(x, h)
  end
  settle(x)
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("SNOWFALL")
  params:add_option("scale", "scale", names, 11)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 60, 84, 72, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("rate", "snowfall", controlspec.new(0.5, 12, 'exp', 0, 3, '/s'))
  params:add_control("wind", "wind", controlspec.new(-1, 1, 'lin', 0, 0.2, ''))
  params:add_control("space", "bell spacing", controlspec.new(0.1, 2, 'exp', 0, 0.45, 's'))
  params:add_control("bells", "bell tone", controlspec.new(800, 9000, 'exp', 0, 4200, 'hz'))
  params:default()
  engine.gain(1)
  math.randomseed(os.time())
  build_scale()
  new_ground()
  -- the first few flakes are already nearly down
  for i = 1, 6 do spawn(10 + i * 18, GROUND - 9 - i * 3) end
  -- and one has just come to rest on a crest
  bell(64, ground[64] + 1)
  midi.connect().event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      local x = util.clamp((msg.note - 60) * 9 + 10, 4, W - 4)
      for _ = 1, 5 do spawn(x + math.random(-6, 6), GROUND - 30 + math.random(0, 10)) end
    end
  end
  local m = metro.init(step, 1 / 30)
  m:start()
end

function step()
  local dt = 1 / 30
  t = t + dt
  gust = gust * 0.97
  if math.random() < params:get("rate") * dt and #flakes < 80 then spawn() end
  local wind = params:get("wind") + gust
  for i = #flakes, 1, -1 do
    local f = flakes[i]
    f.ph = f.ph + 0.08
    f.x = f.x + wind * 0.6 + math.sin(f.ph) * 0.3
    f.y = f.y + f.vy
    if f.x < 0 then f.x = f.x + W elseif f.x >= W then f.x = f.x - W end
    local gx = util.clamp(math.floor(f.x) + 1, 1, W)
    if f.y >= GROUND - ground[gx] then
      land(f)
      table.remove(flakes, i)
    end
  end
  -- a slow thaw keeps the drifts from burying the screen
  local top = 0
  for x = 1, W do top = math.max(top, ground[x]) end
  if top > 42 then
    for x = 1, W do ground[x] = math.max(2, ground[x] - 1) end
  end
  for i = #glints, 1, -1 do
    glints[i].r = glints[i].r + 0.4
    if glints[i].r > 7 then table.remove(glints, i) end
  end
  redraw()
end

function enc(n, d)
  if n == 2 then params:delta("rate", d)
  elseif n == 3 then params:delta("wind", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    gust = (math.random() < 0.5 and -1 or 1) * 1.5
    for _ = 1, 4 do spawn() end
  elseif n == 3 then
    for x = 1, W do ground[x] = math.max(2, math.floor(ground[x] * 0.6)) end
  end
end

function redraw()
  screen.clear()
  screen.line_width(1)
  -- the drifts, solid from the ground up with a bright crust
  for x = 1, W do
    local h = ground[x]
    screen.level(3)
    screen.move(x - 0.5, GROUND + 1)
    screen.line(x - 0.5, GROUND - h + 2)
    screen.stroke()
    screen.level(10)
    screen.pixel(x - 1, GROUND - h)
    screen.fill()
  end
  for _, f in ipairs(flakes) do
    screen.level(f.big and 15 or 8)
    screen.pixel(math.floor(f.x), math.floor(f.y))
    screen.fill()
    if f.big then
      screen.pixel(math.floor(f.x) + 1, math.floor(f.y))
      screen.fill()
    end
  end
  for _, g in ipairs(glints) do
    screen.level(math.max(1, 12 - math.floor(g.r * 1.6)))
    screen.circle(g.x, g.y, g.r)
    screen.stroke()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text("snowfall")
  screen.level(4)
  screen.move(127, 8)
  local w = params:get("wind")
  screen.text_right((w < -0.05 and "< " or (w > 0.05 and "> " or "- ")) .. params:string("rate"))
  screen.update()
end
