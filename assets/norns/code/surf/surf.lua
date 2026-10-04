-- surf
-- a Portamax norns script
--
-- swells roll in from the open sea,
-- steepen, and break on the beach.
-- each break is a deep note from
-- the swell's size, then a spray of
-- bright notes scattered after it.
-- big sets come in threes.
--
-- E2 swell period   E3 spray
-- K2 call a set   K3 calm sea
-- pads: a gull overhead
-- (params: scale, root, tone)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local SHORE = 22
local swells = {}
local spray = {}
local scale = {}
local calm = false
local wait = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 18)
end

local function swell(x, size) table.insert(swells, { x = x, size = size }) end

local function break_wave(w)
  local deg = util.clamp(math.floor(4 - w.size * 3), 1, 3)
  engine.pan(-0.3)
  engine.pw(0.5)
  engine.release(2.5 + w.size * 2)
  engine.cutoff(params:get("tone") * 0.5)
  engine.hz(MusicUtil.note_num_to_freq(scale[deg]))
  local n = math.floor(params:get("spray") * (1 + w.size * 3) + 0.5)
  clock.run(function()
    engine.cutoff(params:get("tone"))
    for i = 1, n do
      clock.sleep(0.04 + math.random() * 0.12)
      engine.pan(-0.6 + math.random() * 1.2)
      engine.pw(0.15)
      engine.release(0.3 + math.random() * 0.5)
      engine.hz(MusicUtil.note_num_to_freq(scale[math.random(8, #scale)] + 12))
      table.insert(spray, { x = SHORE + math.random(0, 14), y = 40 - math.random(0, 18), life = 10 })
    end
  end)
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("SURF")
  params:add_option("scale", "scale", names, 11)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 60, 45, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("period", "swell period", controlspec.new(1.5, 12, 'exp', 0, 4, 's'))
  params:add_number("spray", "spray", 0, 6, 3)
  params:add_control("tone", "tone", controlspec.new(600, 8000, 'exp', 0, 3200, 'hz'))
  params:set_action("tone", function(x) engine.cutoff(x) end)
  params:default()
  engine.amp(0.22)
  math.randomseed(os.time())
  build_scale()
  swell(SHORE + 1, 0.7) -- the first wave is already arriving
  swell(90, 0.5)
  wait = params:get("period")
  midi.connect().event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      engine.pw(0.3) engine.release(0.8) engine.pan(0.5)
      engine.hz(MusicUtil.note_num_to_freq(MusicUtil.snap_note_to_array(msg.note + 12, scale)))
    end
  end
  local dt = 1 / 30
  metro.init(function()
    wait = wait - dt
    if wait <= 0 and not calm then
      wait = params:get("period") * (0.8 + math.random() * 0.4)
      swell(132, 0.3 + math.random() * 0.4)
    end
    for i = #swells, 1, -1 do
      local w = swells[i]
      w.x = w.x - dt * 30 * (calm and 0.5 or 1)
      if w.x <= SHORE then break_wave(w) table.remove(swells, i) end
    end
    for i = #spray, 1, -1 do
      spray[i].life = spray[i].life - 1
      if spray[i].life <= 0 then table.remove(spray, i) end
    end
    redraw()
  end, dt):start()
end

function enc(n, d)
  if n == 2 then params:delta("period", d)
  elseif n == 3 then params:delta("spray", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    -- a set: three big ones, the first right at the break
    swell(SHORE + 2, 1) swell(SHORE + 40, 0.9) swell(SHORE + 80, 0.95)
  elseif n == 3 then calm = not calm end
end

function redraw()
  screen.clear()
  -- beach
  screen.level(5)
  screen.move(0, 54)
  screen.line(SHORE, 44)
  screen.stroke()
  -- sea surface with the swells as bumps that steepen near the shore
  screen.level(8)
  local first = true
  for x = SHORE, 127, 2 do
    local y = 44
    for _, w in ipairs(swells) do
      local d = x - w.x
      local width = 6 + (w.x - SHORE) / 8
      y = y - w.size * (8 + 6 * (1 - (w.x - SHORE) / 110)) * math.exp(-(d * d) / (width * width))
    end
    if first then screen.move(x, y) first = false else screen.line(x, y) end
  end
  screen.stroke()
  screen.level(2)
  for x = SHORE + 4, 127, 6 do screen.pixel(x, 50 + (x % 4)) screen.fill() end
  for _, s in ipairs(spray) do
    screen.level(s.life + 3)
    screen.pixel(s.x, s.y)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text(calm and "surf (calm)" or "surf")
  screen.level(4)
  screen.move(0, 62)
  screen.text("swell " .. params:string("period"))
  screen.move(127, 62)
  screen.text_right("spray " .. params:get("spray"))
  screen.update()
end
