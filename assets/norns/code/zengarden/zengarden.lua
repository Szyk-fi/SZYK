-- zengarden
-- a Portamax norns script
--
-- raked sand around a few stones:
-- straight furrows across the garden,
-- rings around each stone. a slow
-- rake wanders over it, and every
-- furrow it crosses sounds. the rings
-- of each stone have their own chord.
--
-- E2 rake speed   E3 rings per stone
-- K2 new garden   K3 pause
-- pads: tap a stone
-- (params: scale, root, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local SPACING = 5
local stones = {}
local scale = {}
local rake = { x = 64, y = 34, t = 0, region = nil }
local trail = {}
local paused = false

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 22)
end

local function new_garden()
  stones = {}
  for i = 1, 3 do
    stones[i] = { x = 20 + (i - 1) * 42 + math.random(-6, 6), y = math.random(24, 44),
      r = math.random(3, 5), deg = math.random(1, 7), lit = 0 }
  end
end

local function region(x, y)
  -- which furrow is under the rake: a ring of a stone, or a straight line
  local rings = params:get("rings")
  for i, s in ipairs(stones) do
    local d = math.sqrt((x - s.x) ^ 2 + (y - s.y) ^ 2)
    if d < s.r + rings * SPACING then return i * 100 + math.floor((d - s.r) / SPACING), i end
  end
  return math.floor(y / SPACING), nil
end

local function play(n, amp, x)
  engine.amp(amp)
  engine.pan(util.linlin(0, 128, -0.6, 0.6, x))
  engine.hz(MusicUtil.note_num_to_freq(n))
end

local function cross(id, si)
  if si then
    local s = stones[si]
    local ring = id % 100
    play(scale[util.clamp(s.deg + ring * 2, 1, #scale)], 0.24, s.x)
    s.lit = 6
  else
    play(scale[util.clamp(12 - id, 1, #scale)], 0.15, rake.x)
  end
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("ZENGARDEN")
  params:add_option("scale", "scale", names, 38)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 40, 64, 50, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("speed", "rake speed", controlspec.new(0.3, 3, 'exp', 0, 1, 'x'))
  params:add_number("rings", "rings per stone", 1, 5, 3)
  params:add_control("release", "release", controlspec.new(0.3, 4, 'exp', 0, 1.8, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:default()
  engine.cutoff(1600)
  engine.pw(0.45)
  math.randomseed(os.time())
  build_scale()
  new_garden()
  rake.region = region(rake.x, rake.y)
  play(scale[8], 0.2, 64)
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      local si = (msg.note - 60) % #stones + 1
      cross(si * 100 + (msg.note % 3), si)
    end
  end
  clock.run(function()
    while true do
      clock.sleep(1 / 30)
      if not paused then move() end
      for _, s in ipairs(stones) do s.lit = math.max(0, s.lit - 1) end
      redraw()
    end
  end)
end

function move()
  -- a slow lissajous wander, like an unhurried hand
  rake.t = rake.t + params:get("speed") / 30 * 0.25
  rake.x = 64 + 58 * math.sin(rake.t * 0.7)
  rake.y = 34 + 19 * math.sin(rake.t * 1.3 + 0.5)
  local id, si = region(rake.x, rake.y)
  if id ~= rake.region then rake.region = id cross(id, si) end
  table.insert(trail, { rake.x, rake.y })
  if #trail > 24 then table.remove(trail, 1) end
end

function enc(n, d)
  if n == 2 then params:delta("speed", d)
  elseif n == 3 then params:delta("rings", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_garden() cross(100, 1)
  elseif n == 3 then paused = not paused end
  redraw()
end

function redraw()
  screen.clear()
  local rings = params:get("rings")
  screen.level(2)
  for y = 15, 54, SPACING do
    for x = 0, 127, 2 do
      local _, si = region(x, y)
      if not si then screen.pixel(x, y) screen.fill() end
    end
  end
  for _, s in ipairs(stones) do
    for k = 1, rings do
      screen.level(s.lit > 0 and 8 or 3)
      screen.circle(s.x, s.y, s.r + k * SPACING - 1) screen.stroke()
    end
    screen.level(s.lit > 0 and 15 or 10)
    screen.circle(s.x, s.y, s.r) screen.fill()
  end
  for i, p in ipairs(trail) do
    screen.level(math.max(1, i // 3))
    screen.pixel(p[1], p[2]) screen.fill()
  end
  screen.level(15)
  screen.move(rake.x - 3, rake.y) screen.line(rake.x + 3, rake.y) screen.stroke()
  screen.move(0, 8)
  screen.text(paused and "zengarden (paused)" or "zengarden")
  screen.level(4)
  screen.move(0, 62)
  screen.text("rake " .. params:string("speed"))
  screen.move(127, 62)
  screen.text_right(rings .. " rings")
  screen.update()
end
