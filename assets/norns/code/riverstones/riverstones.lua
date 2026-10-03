-- riverstones
-- a Portamax norns script
--
-- water runs left to right over
-- a bed of stones. swells travel
-- downstream; each time the water
-- crests a stone, that stone
-- pings its pitch and a ripple
-- spreads away on the current.
--
-- E2 flow       E3 brightness
-- K2 new stones K3 skip a pebble
-- pads: drop a stone in the river
-- (params: scale, root, swell)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local BED = 60
local stones = {}
local foam = {}
local ripples = {}
local scale = {}
local t = 0
local pebble = nil

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 14)
end

-- water depth above the bed at x: a mean level plus two swells
-- moving downstream at different speeds
local function water(x)
  local f = params:get("flow")
  local a = params:get("swell")
  return 17 + a * (math.sin(x * 0.07 - t * 1.3 * f) * 6 + math.sin(x * 0.19 - t * 2.9 * f + 1.7) * 3)
end

local function ping(i, amp)
  local s = stones[i]
  engine.pan((s.x / 128 - 0.5) * 1.6)
  engine.amp(amp)
  engine.pw(0.3 + s.h / 60)
  engine.release(0.6 + s.r * 0.12)
  engine.cutoff(params:get("bright"))
  engine.hz(MusicUtil.note_num_to_freq(scale[s.deg]))
  s.flash = 15
  table.insert(ripples, { x = s.x, y = BED - water(s.x), r = 1 })
end

local function new_stones()
  stones = {}
  local x = 8
  while x < 120 do
    local r = math.random(3, 7)
    local h = math.random(8, 24)
    table.insert(stones, { x = x + r, r = r, h = h, wet = false, flash = 0 })
    x = x + r * 2 + math.random(5, 14)
  end
  for _, s in ipairs(stones) do
    -- taller stones are reached less often and sing higher
    s.deg = util.clamp(math.floor(util.linlin(8, 24, 1, #scale - 2, s.h)) + math.random(0, 2), 1, #scale)
    s.wet = water(s.x) > s.h
  end
end

local function skip_pebble()
  pebble = { x = 0, y = BED - 30, vy = -1.5, hops = 0 }
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("RIVERSTONES")
  params:add_option("scale", "scale", names, 5)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 50, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("flow", "flow", controlspec.new(0.2, 3, 'exp', 0, 1, 'x'))
  params:add_control("swell", "swell", controlspec.new(0.3, 1.5, 'lin', 0, 1, 'x'))
  params:add_control("bright", "brightness", controlspec.new(300, 8000, 'exp', 0, 2400, 'hz'))
  params:default()
  engine.gain(1.2)
  math.randomseed(os.time())
  build_scale()
  new_stones()
  for i = 1, 24 do foam[i] = { x = math.random() * 128, o = math.random() * 3 } end
  midi.connect().event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      local i = ((msg.note % #stones) + 1)
      ping(i, 0.3)
    end
  end
  skip_pebble()
  local m = metro.init(step, 1 / 30)
  m:start()
end

function step()
  t = t + 1 / 30
  for i, s in ipairs(stones) do
    local wet = water(s.x) > s.h
    if wet and not s.wet then ping(i, 0.16 + (24 - s.h) * 0.006) end
    s.wet = wet
    s.flash = math.max(0, s.flash - 1)
  end
  local f = params:get("flow")
  for _, p in ipairs(foam) do
    p.x = p.x + 0.6 * f + math.random() * 0.3
    if p.x > 128 then p.x = 0 end
  end
  for i = #ripples, 1, -1 do
    local r = ripples[i]
    r.r = r.r + 0.5
    r.x = r.x + 0.5 * f
    if r.r > 10 then table.remove(ripples, i) end
  end
  if pebble then
    pebble.x = pebble.x + 3
    pebble.vy = pebble.vy + 0.35
    pebble.y = pebble.y + pebble.vy
    local surface = BED - water(pebble.x)
    if pebble.y >= surface then
      pebble.y = surface
      pebble.vy = -2.6 + pebble.hops * 0.45
      pebble.hops = pebble.hops + 1
      -- each hop pings whichever stone lies beneath it
      local best, bd = 1, 999
      for i, s in ipairs(stones) do
        local d = math.abs(s.x - pebble.x)
        if d < bd then best, bd = i, d end
      end
      ping(best, 0.22 - pebble.hops * 0.025)
      if pebble.hops > 5 then pebble = nil end
    end
    if pebble and pebble.x > 128 then pebble = nil end
  end
  redraw()
end

function enc(n, d)
  if n == 2 then params:delta("flow", d)
  elseif n == 3 then params:delta("bright", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_stones() ping(math.random(1, #stones), 0.2)
  elseif n == 3 then skip_pebble() end
end

function redraw()
  screen.clear()
  screen.line_width(1)
  -- riverbed
  screen.level(2)
  screen.move(0, BED + 1)
  screen.line(128, BED + 1)
  screen.stroke()
  -- stones
  for _, s in ipairs(stones) do
    screen.level(s.flash > 0 and math.max(6, s.flash) or (s.wet and 4 or 7))
    screen.move(s.x - s.r, BED)
    screen.curve(s.x - s.r, BED - s.h * 1.3, s.x + s.r, BED - s.h * 1.3, s.x + s.r, BED)
    screen.close()
    screen.fill()
  end
  -- the water surface
  screen.level(10)
  screen.move(0, BED - water(0))
  for x = 2, 128, 2 do screen.line(x, BED - water(x)) end
  screen.stroke()
  for _, p in ipairs(foam) do
    screen.level(5)
    screen.pixel(math.floor(p.x), math.floor(BED - water(p.x) + 2 + p.o))
    screen.fill()
  end
  for _, r in ipairs(ripples) do
    screen.level(math.max(1, 13 - math.floor(r.r)))
    screen.arc(r.x, r.y, r.r, math.pi * 1.1, math.pi * 1.9)
    screen.stroke()
  end
  if pebble then
    screen.level(15)
    screen.circle(pebble.x, pebble.y, 1.5)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text("riverstones")
  screen.level(4)
  screen.move(127, 8)
  screen.text_right("flow " .. params:string("flow"))
  screen.update()
end
