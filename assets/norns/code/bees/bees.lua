-- bees
-- a Portamax norns script
--
-- bees forage a meadow of flowers,
-- each flower a pitch. a bee that
-- lands hums its flower softly;
-- back at the hive it dances, and
-- the waggle plays a short motif
-- of where it went. a good dance
-- sends more bees to that flower.
--
-- E2 bees       E3 brightness
-- K2 new meadow K3 bloom (refill)
-- pads: a bee dances that flower
-- (params: scale, root)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local HIVE = { x = 14, y = 50 }
local flowers = {}
local bees = {}
local scale = {}
local dances = {}
local t = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 15)
end

local function play(deg, amp, pan, rel, pw)
  engine.pan(pan)
  engine.amp(amp)
  engine.pw(pw or 0.4)
  engine.release(rel)
  engine.cutoff(params:get("bright"))
  engine.hz(MusicUtil.note_num_to_freq(scale[util.clamp(deg, 1, #scale)]))
end

local function new_meadow()
  flowers = {}
  local degs = { 3, 5, 6, 8, 10, 12, 13 }
  for i = 1, 7 do
    local x, y
    repeat
      x = 34 + math.random() * 88
      y = 16 + math.random() * 42
      local ok = true
      for _, f in ipairs(flowers) do
        if math.abs(f.x - x) + math.abs(f.y - y) < 16 then ok = false end
      end
    until ok
    flowers[i] = { x = x, y = y, deg = degs[i], nectar = 1, fame = 1, petals = math.random(4, 6), lit = 0 }
  end
end

-- a forager picks a flower weighted by fame (recruitment) and nectar
local function choose()
  local total = 0
  for _, f in ipairs(flowers) do total = total + f.fame * (0.2 + f.nectar) end
  local r = math.random() * total
  for i, f in ipairs(flowers) do
    r = r - f.fame * (0.2 + f.nectar)
    if r <= 0 then return i end
  end
  return #flowers
end

local function new_bee()
  return { x = HIVE.x, y = HIVE.y, vx = 0, vy = 0, state = "out", target = choose(), route = {}, wait = 0 }
end

-- the waggle: the first note is the flower, then steps whose size
-- follows the bearing and whose count follows the distance
local function dance(route, x, y)
  if #route == 0 then return end
  local d = { x = x, y = y, age = 0, len = 1 }
  table.insert(dances, d)
  clock.run(function()
    for k, fi in ipairs(route) do
      local f = flowers[fi]
      if f then
        local dist = math.sqrt((f.x - HIVE.x) ^ 2 + (f.y - HIVE.y) ^ 2)
        local bearing = math.atan(f.y - HIVE.y, f.x - HIVE.x)
        local waggles = util.clamp(math.floor(dist / 25) + 1, 1, 4)
        local stepdir = bearing < -0.3 and 1 or (bearing > 0.3 and -1 or 2)
        local deg = f.deg
        for w = 1, waggles do
          play(deg, 0.2, (f.x / 128 - 0.5), 0.35, 0.55)
          f.lit = 10
          d.len = w + k
          deg = deg + stepdir
          clock.sleep(0.11)
        end
        clock.sleep(0.08)
      end
    end
  end)
end

local function set_bee_count(n)
  while #bees < n do table.insert(bees, new_bee()) end
  while #bees > n do table.remove(bees) end
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("BEES")
  params:add_option("scale", "scale", names, 7)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 48, 72, 57, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_number("count", "bees", 1, 10, 5)
  params:set_action("count", function(n) set_bee_count(n) end)
  params:add_control("bright", "brightness", controlspec.new(300, 8000, 'exp', 0, 2600, 'hz'))
  math.randomseed(os.time())
  new_meadow()
  params:default()
  engine.gain(1.1)
  build_scale()
  -- a scout is already home from a trip, ready to dance
  dance({ 2, 5 }, HIVE.x + 6, HIVE.y - 8)
  midi.connect().event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      local fi = (msg.note % #flowers) + 1
      flowers[fi].fame = flowers[fi].fame + 1
      dance({ fi }, HIVE.x + 6, HIVE.y - 8)
    end
  end
  local m = metro.init(step, 1 / 30)
  m:start()
end

local function steer(b, tx, ty, speed)
  local dx, dy = tx - b.x, ty - b.y
  local d = math.sqrt(dx * dx + dy * dy) + 0.001
  b.vx = b.vx * 0.85 + dx / d * speed * 0.15 + (math.random() - 0.5) * 0.5
  b.vy = b.vy * 0.85 + dy / d * speed * 0.15 + (math.random() - 0.5) * 0.5
  b.x = b.x + b.vx
  b.y = b.y + b.vy
  return d
end

function step()
  t = t + 1 / 30
  for _, b in ipairs(bees) do
    if b.state == "out" then
      local f = flowers[b.target]
      if not f then b.target = choose() f = flowers[b.target] end
      if steer(b, f.x, f.y, 1.6) < 2.5 then
        b.state = "sip"
        b.wait = 10 + math.random(0, 15)
        local got = math.min(f.nectar, 0.25)
        f.nectar = f.nectar - got
        table.insert(b.route, b.target)
        f.lit = 8
        play(f.deg - 7, 0.06 + got * 0.3, (f.x / 128 - 0.5), 0.9, 0.2)
      end
    elseif b.state == "sip" then
      b.wait = b.wait - 1
      if b.wait <= 0 then
        -- a full load goes home; otherwise try one more flower
        if #b.route >= 2 or math.random() < 0.5 then b.state = "home"
        else b.state = "out" b.target = choose() end
      end
    elseif b.state == "home" then
      if steer(b, HIVE.x, HIVE.y, 1.8) < 3 then
        local rich = 0
        for _, fi in ipairs(b.route) do
          flowers[fi].fame = math.min(6, flowers[fi].fame + 0.6)
          rich = rich + flowers[fi].nectar
        end
        if rich > 0.2 or math.random() < 0.4 then dance(b.route, b.x + 4, b.y - 6) end
        b.route = {}
        b.state = "rest"
        b.wait = 20 + math.random(0, 40)
      end
    elseif b.state == "rest" then
      b.wait = b.wait - 1
      if b.wait <= 0 then b.state = "out" b.target = choose() end
    end
  end
  for _, f in ipairs(flowers) do
    f.nectar = math.min(1, f.nectar + 0.002)
    f.fame = math.max(1, f.fame - 0.002)
    f.lit = math.max(0, f.lit - 0.5)
  end
  for i = #dances, 1, -1 do
    dances[i].age = dances[i].age + 1
    if dances[i].age > 40 then table.remove(dances, i) end
  end
  redraw()
end

function enc(n, d)
  if n == 2 then params:delta("count", d)
  elseif n == 3 then params:delta("bright", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    new_meadow()
    for _, b in ipairs(bees) do b.route = {} b.state = "out" b.target = choose() end
  elseif n == 3 then
    for _, f in ipairs(flowers) do f.nectar = 1 f.lit = 6 end
    play(1, 0.18, 0, 1.6, 0.5)
  end
end

function redraw()
  screen.clear()
  screen.line_width(1)
  -- the hive: a little stack of comb
  screen.level(6)
  for r = 0, 2 do
    screen.rect(HIVE.x - 6 + r, HIVE.y - 4 + r * 4, 12 - r * 2, 3)
    screen.fill()
  end
  screen.level(0)
  screen.rect(HIVE.x - 1, HIVE.y + 6, 2, 2)
  screen.fill()
  for _, f in ipairs(flowers) do
    local lv = math.floor(3 + f.nectar * 5 + f.lit)
    screen.level(math.min(15, lv))
    for p = 1, f.petals do
      local a = p / f.petals * 2 * math.pi + t * 0.2
      screen.pixel(math.floor(f.x + math.cos(a) * 3), math.floor(f.y + math.sin(a) * 3))
      screen.fill()
    end
    screen.level(math.min(15, 6 + math.floor(f.lit)))
    screen.circle(f.x, f.y, 1)
    screen.fill()
    screen.level(2)
    screen.move(f.x, f.y + 4)
    screen.line(f.x, math.min(63, f.y + 8))
    screen.stroke()
  end
  for _, b in ipairs(bees) do
    if b.state ~= "rest" then
      screen.level(15)
      screen.pixel(math.floor(b.x), math.floor(b.y))
      screen.fill()
      screen.level(5)
      screen.pixel(math.floor(b.x - b.vx), math.floor(b.y - b.vy))
      screen.fill()
    end
  end
  -- waggle runs: a zigzag figure-eight beside the hive
  for _, d in ipairs(dances) do
    screen.level(math.max(1, 12 - math.floor(d.age / 4)))
    screen.move(d.x, d.y)
    for k = 1, d.len * 2 do
      screen.line(d.x + k * 1.5, d.y + ((k % 2 == 0) and -1.5 or 1.5))
    end
    screen.stroke()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text("bees")
  screen.level(4)
  screen.move(127, 8)
  screen.text_right(#bees .. " foraging")
  screen.update()
end
