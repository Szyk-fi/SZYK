-- warble
-- a Portamax norns script
--
-- a slow melody into a worn-out
-- tape echo. the echo's speed
-- wanders like a stretched tape,
-- so every repeat bends in pitch.
--
-- E2 wow   E3 repeats
-- K2 new melody   K3 stop tape
-- (params: root, tone)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local melody = {}
local idx = 0
local wow_phase = 0
local rate = 1
local tape_on = true

local function new_melody()
  local s = MusicUtil.generate_scale_of_length(params:get("root"), "dorian", 10)
  melody = {}
  for i = 1, 8 do melody[i] = math.random() < 0.25 and 0 or s[math.random(#s)] end
end

local function setup_tape()
  audio.level_eng_cut(1)
  softcut.buffer_clear()
  softcut.enable(1, 1)
  softcut.buffer(1, 1)
  softcut.level(1, 0.75)
  softcut.pan(1, 0.2)
  softcut.rate(1, 1)
  softcut.loop(1, 1)
  softcut.loop_start(1, 1)
  softcut.loop_end(1, 1.6)
  softcut.position(1, 1)
  softcut.fade_time(1, 0.04)
  softcut.level_input_cut(1, 1, 1.0)
  softcut.level_input_cut(2, 1, 1.0)
  softcut.rec_level(1, 1)
  softcut.pre_level(1, 0.7)
  softcut.play(1, 1)
  softcut.rec(1, 1)
  softcut.rate_slew_time(1, 0.2)
  softcut.filter_dry(1, 0)
  softcut.filter_lp(1, 1)
  softcut.filter_fc(1, 2000)
  softcut.filter_rq(1, 2)
end

function init()
  params:add_separator("WARBLE")
  params:add_number("root", "root", 48, 72, 57, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:add_control("wow", "wow", controlspec.new(0, 0.08, 'lin', 0, 0.02, ''))
  params:add_control("repeats", "repeats", controlspec.new(0, 0.95, 'lin', 0, 0.7, ''))
  params:set_action("repeats", function(x) softcut.pre_level(1, x) end)
  params:add_control("tone", "tone", controlspec.new(400, 5000, 'exp', 0, 1500, 'hz'))
  params:set_action("tone", function(x) engine.cutoff(x) end)
  params:default()
  engine.release(1.0)
  engine.amp(0.3)
  math.randomseed(os.time())
  setup_tape()
  new_melody()
  clock.run(function()
    while true do
      clock.sync(1 / 2)
      idx = idx % #melody + 1
      if melody[idx] > 0 then engine.hz(MusicUtil.note_num_to_freq(melody[idx])) end
    end
  end)
  local m = metro.init(function()
    wow_phase = wow_phase + 1 / 30
    -- two slow sines of different speeds: wow plus a slower drift
    rate = 1 + params:get("wow") * (math.sin(wow_phase * 2.1) + 0.6 * math.sin(wow_phase * 0.37))
    softcut.rate(1, tape_on and rate or 0)
    redraw()
  end, 1 / 30)
  m:start()
end

function enc(n, d)
  if n == 2 then params:delta("wow", d)
  elseif n == 3 then params:delta("repeats", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_melody() elseif n == 3 then tape_on = not tape_on end
end

function redraw()
  screen.clear()
  -- two tape reels turning at the tape's speed
  for i, cx in ipairs({ 40, 88 }) do
    screen.level(6)
    screen.circle(cx, 34, 14)
    screen.stroke()
    local a = wow_phase * 4 * (tape_on and rate or 0) + i
    screen.level(15)
    screen.move(cx, 34)
    screen.line(cx + math.cos(a) * 12, 34 + math.sin(a) * 12)
    screen.stroke()
  end
  for i, n in ipairs(melody) do
    screen.level(i == idx and 15 or 3)
    screen.rect(30 + i * 7, 58, 5, n > 0 and 3 or 1)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 7)
  screen.text(tape_on and "warble" or "warble (tape stopped)")
  screen.level(4)
  screen.move(127, 7)
  screen.text_right(string.format("x%.3f", rate))
  screen.update()
end

function cleanup()
  softcut.rec(1, 0)
end
