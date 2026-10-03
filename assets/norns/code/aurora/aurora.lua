-- aurora
-- a Portamax norns script
--
-- curtains of light drift and fold
-- across the night sky. a slow pad
-- walks through a chord cycle while
-- softcut holds every note in a long
-- wash, with a second head reading
-- it an octave up for shimmer.
--
-- E2 drift   E3 shimmer
-- K2 new chord cycle   K3 hold
-- pads: add a star note
-- (params: scale, root, wash)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local scale = {}
local cycle = { 1, 6, 4, 5 }
local chord_i, beat = 1, 0
local held = false
local t = 0
local pulse = 0
local stars = {}

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 22)
end

local function setup_wash()
  audio.level_eng_cut(0.7)
  softcut.buffer_clear()
  for v = 1, 2 do
    softcut.enable(v, 1) softcut.buffer(v, 1) softcut.loop(v, 1)
    softcut.loop_start(v, 1) softcut.loop_end(v, 7)
    softcut.position(v, v == 1 and 1 or 4)
    softcut.fade_time(v, 0.2) softcut.play(v, 1)
    softcut.filter_dry(v, 0) softcut.filter_lp(v, 1)
    softcut.filter_fc(v, v == 1 and 2500 or 4000)
    softcut.filter_rq(v, 2)
  end
  softcut.level_input_cut(1, 1, 1)
  softcut.level_input_cut(2, 1, 1)
  softcut.rec_level(1, 1)
  softcut.rec(1, 1)
  -- head 1 records and repeats; head 2 reads the same tape an octave up
  softcut.rate(1, 1) softcut.level(1, 0.6) softcut.pan(1, -0.4)
  softcut.rate(2, 2) softcut.pan(2, 0.5)
end

local function pad_note()
  local deg = cycle[chord_i] + ({ 0, 2, 4, 7, 9 })[math.random(1, 5)]
  engine.pan((math.random() - 0.5) * 1.2)
  engine.pw(0.4 + math.random() * 0.2)
  engine.release(3.5)
  engine.hz(MusicUtil.note_num_to_freq(scale[util.clamp(deg, 1, #scale)]))
  pulse = 10
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("AURORA")
  params:add_option("scale", "scale", names, 7)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 36, 60, 45, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_control("drift", "drift", controlspec.new(0.1, 3, 'exp', 0, 0.8, 'x'))
  params:add_control("shimmer", "shimmer", controlspec.new(0, 1, 'lin', 0.01, 0.35, ''))
  params:set_action("shimmer", function(x) softcut.level(2, x) end)
  params:add_control("wash", "wash", controlspec.new(0, 0.95, 'lin', 0, 0.75, ''))
  params:set_action("wash", function(x) softcut.pre_level(1, x) end)
  params:add_control("bright", "brightness", controlspec.new(200, 4000, 'exp', 0, 1100, 'hz'))
  params:set_action("bright", function(x) engine.cutoff(x) end)
  setup_wash()
  params:default()
  engine.amp(0.2)
  math.randomseed(os.time())
  build_scale()
  pad_note()
  for i = 1, 14 do stars[i] = { math.random(0, 127), math.random(11, 30) } end
  midi.connect().event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then engine.release(2) engine.hz(MusicUtil.note_num_to_freq(msg.note + 12)) end
  end
  clock.run(function()
    while true do
      clock.sync(1 / 2)
      if not held then
        beat = beat + 1
        if beat % 16 == 0 then chord_i = chord_i % #cycle + 1 end
        if beat % 2 == 0 or math.random() < 0.3 then pad_note() end
      end
    end
  end)
  metro.init(function()
    t = t + params:get("drift") / 30
    pulse = math.max(0, pulse - 0.3)
    redraw()
  end, 1 / 30):start()
end

function enc(n, d)
  if n == 2 then params:delta("drift", d)
  elseif n == 3 then params:delta("shimmer", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    cycle = { 1 }
    for i = 2, 4 do cycle[i] = math.random(2, 7) end
    chord_i = 1
    pad_note()
  elseif n == 3 then held = not held end
end

function redraw()
  screen.clear()
  screen.level(3)
  for _, s in ipairs(stars) do screen.pixel(s[1], s[2]) screen.fill() end
  -- three curtains, each a row of hanging rays
  for c = 1, 3 do
    for x = 0, 127, 2 do
      local top = 16 + c * 4 + math.sin(x / 17 + t * c * 0.7) * 5 + math.sin(x / 7 - t * 1.3) * 2
      local len = 10 + math.sin(x / 11 + t + c) * 6 + pulse
      local lvl = math.floor(2 + (math.sin(x / 9 - t * 2 + c * 2) + 1) * (2 + pulse / 3))
      screen.level(util.clamp(lvl, 1, 15))
      screen.move(x, top)
      screen.line(x, top + len)
      screen.stroke()
    end
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text(held and "aurora (held)" or "aurora")
  screen.level(4)
  screen.move(0, 62)
  screen.text("drift " .. params:string("drift"))
  screen.move(127, 62)
  screen.text_right("shimmer " .. params:string("shimmer"))
  screen.update()
end

function cleanup()
  softcut.rec(1, 0)
end
