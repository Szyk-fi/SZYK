-- tidepool
-- a Portamax norns demo
--
-- two tides rise and fall;
-- where they meet, notes fall
-- into a pool of echoes.
--
-- E2 density   E3 brightness
-- K2 new tide  K3 pause
-- (params: scale, root, echo)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local scale = {}
local tide = { 0, 0 }
local speed = { 0.13, 0.071 }
local drops = {}
local paused = false
local step = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 16)
end

local function setup_echo()
  audio.level_eng_cut(0.6)
  softcut.buffer_clear()
  softcut.enable(1, 1)
  softcut.buffer(1, 1)
  softcut.level(1, 0.6)
  softcut.pan(1, 0.3)
  softcut.rate(1, 1)
  softcut.loop(1, 1)
  softcut.loop_start(1, 2)
  softcut.loop_end(1, 2.75)
  softcut.position(1, 2)
  softcut.fade_time(1, 0.05)
  softcut.level_input_cut(1, 1, 1.0)
  softcut.level_input_cut(2, 1, 1.0)
  softcut.rec_level(1, 1)
  softcut.pre_level(1, 0.6)
  softcut.play(1, 1)
  softcut.rec(1, 1)
  softcut.filter_dry(1, 0)
  softcut.filter_lp(1, 1)
  softcut.filter_fc(1, 2400)
  softcut.filter_rq(1, 1.5)
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("TIDEPOOL")
  params:add_option("scale", "scale", names, 11)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 72, 50, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_number("density", "density", 1, 16, 6)
  params:add_control("bright", "brightness", controlspec.new(200, 6000, 'exp', 0, 1400, 'hz'))
  params:set_action("bright", function(x) engine.cutoff(x) end)
  params:add_control("release", "release", controlspec.new(0.1, 4, 'lin', 0, 1.6, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:add_control("echo", "echo", controlspec.new(0, 0.95, 'lin', 0, 0.6, ''))
  params:set_action("echo", function(x) softcut.pre_level(1, x) end)
  params:default()
  engine.amp(0.25)
  engine.gain(1.5)
  setup_echo()
  build_scale()
  math.randomseed(os.time())
  clock.run(function()
    while true do
      clock.sync(1 / 4)
      if not paused then tick() end
      redraw()
    end
  end)
end

function tick()
  step = step + 1
  for i = 1, 2 do tide[i] = (tide[i] + speed[i]) % (2 * math.pi) end
  local a = (math.sin(tide[1]) + 1) / 2
  local b = (math.sin(tide[2] + 1) + 1) / 2
  -- the closer the tides, the likelier a note
  local chance = (1 - math.abs(a - b)) * params:get("density") / 16
  if math.random() < chance then
    local deg = math.floor(((a + b) / 2) * (#scale - 1)) + 1
    local note = scale[deg]
    engine.pan((a - b) * 1.5)
    engine.pw(0.2 + 0.6 * b)
    engine.hz(MusicUtil.note_num_to_freq(note))
    table.insert(drops, { x = 8 + deg * 7, r = 1 })
  end
  for i = #drops, 1, -1 do
    drops[i].r = drops[i].r + 1
    if drops[i].r > 14 then table.remove(drops, i) end
  end
end

function enc(n, d)
  if n == 2 then params:delta("density", d)
  elseif n == 3 then params:delta("bright", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    speed = { 0.05 + math.random() * 0.15, 0.03 + math.random() * 0.12 }
  elseif n == 3 then
    paused = not paused
  end
  redraw()
end

function redraw()
  screen.clear()
  screen.aa(0)
  screen.line_width(1)
  -- the two tides
  for i = 1, 2 do
    screen.level(i == 1 and 6 or 3)
    for x = 0, 127, 2 do
      local y = 40 + math.sin(tide[i] + (i == 2 and 1 or 0) + x / 20) * 10
      screen.pixel(x, y)
      screen.fill()
    end
  end
  -- ripples
  for _, d in ipairs(drops) do
    screen.level(math.max(1, 15 - d.r))
    screen.circle(d.x, 40, d.r)
    screen.stroke()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text(paused and "tidepool (paused)" or "tidepool")
  screen.level(4)
  screen.move(0, 62)
  screen.text("dens " .. params:get("density"))
  screen.move(127, 62)
  screen.text_right(params:string("bright"))
  screen.update()
end

function cleanup()
  softcut.rec(1, 0)
end
