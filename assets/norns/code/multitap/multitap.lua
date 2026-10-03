-- multitap
-- a Portamax norns script
--
-- one record head, four read heads.
-- each tap reads the tape a set time
-- behind the writer, quieter and
-- further out in the stereo field,
-- so sparse notes bloom into
-- rhythmic clusters.
--
-- E2 tap spacing   E3 tap tone
-- K2 new motif   K3 even / swung taps
-- (params: tap fall-off)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local LOOP = 8
local PANS = { -0.8, 0.7, -0.4, 0.35 }
local motif = {}
local mstep = 0
local swung = false
local times = {}
local lit = { 0, 0, 0, 0, 0 }

local function tap_times()
  local beat = 60 / clock.get_tempo()
  local sp = params:get("spacing")
  for k = 1, 4 do
    local t = k * sp * beat
    if swung and k % 2 == 1 then t = t + sp * beat * 0.33 end
    times[k] = math.min(t, LOOP - 0.1)
  end
end

local function place_heads()
  tap_times()
  softcut.position(1, 1)
  for k = 1, 4 do
    -- a tap sits `times[k]` behind the writer, wrapped into the loop
    softcut.position(k + 1, 1 + (LOOP - times[k]) % LOOP)
  end
end

local function setup()
  audio.level_eng_cut(1)
  softcut.buffer_clear()
  for v = 1, 5 do
    softcut.enable(v, 1)
    softcut.buffer(v, 1)
    softcut.loop(v, 1)
    softcut.loop_start(v, 1)
    softcut.loop_end(v, 1 + LOOP)
    softcut.rate(v, 1)
    softcut.fade_time(v, 0.01)
    softcut.play(v, 1)
    softcut.rec(v, 0)
  end
  softcut.level(1, 0)
  softcut.level_input_cut(1, 1, 1.0)
  softcut.level_input_cut(2, 1, 1.0)
  softcut.rec_level(1, 1)
  softcut.pre_level(1, 0)
  softcut.rec(1, 1)
  for k = 1, 4 do
    softcut.pan(k + 1, PANS[k])
    softcut.filter_dry(k + 1, 0)
    softcut.filter_lp(k + 1, 1)
  end
end

local function levels()
  for k = 1, 4 do
    softcut.level(k + 1, 0.8 * params:get("falloff") ^ (k - 1))
    softcut.filter_fc(k + 1, params:get("tone") / (1 + (k - 1) * 0.4))
  end
end

local function new_motif()
  local s = MusicUtil.generate_scale_of_length(60, "Mixolydian", 10)
  motif = {}
  for i = 1, 8 do motif[i] = (i == 1 or math.random() < 0.35) and s[math.random(#s)] or 0 end
end

function init()
  params:add_separator("MULTITAP")
  params:add_control("spacing", "tap spacing", controlspec.new(0.25, 1.5, 'lin', 0.125, 0.75, 'beats'))
  params:set_action("spacing", function() place_heads() end)
  params:add_control("tone", "tap tone", controlspec.new(500, 8000, 'exp', 0, 3000, 'hz'))
  params:set_action("tone", levels)
  params:add_control("falloff", "tap fall-off", controlspec.new(0.3, 0.95, 'lin', 0, 0.7, ''))
  params:set_action("falloff", levels)
  setup()
  params:default()
  engine.release(0.5)
  engine.amp(0.3)
  engine.cutoff(2500)
  math.randomseed(os.time())
  new_motif()
  clock.run(function()
    while true do
      clock.sync(1 / 2)
      mstep = mstep % #motif + 1
      local n = motif[mstep]
      if n > 0 then
        engine.hz(MusicUtil.note_num_to_freq(n))
        lit[1] = 15
        for k = 1, 4 do
          clock.run(function() clock.sleep(times[k]) lit[k + 1] = 15 end)
        end
      end
    end
  end)
  clock.run(function()
    while true do
      clock.sleep(1 / 20)
      for i = 1, 5 do lit[i] = math.max(0, lit[i] - 1) end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("spacing", d)
  elseif n == 3 then params:delta("tone", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_motif()
  elseif n == 3 then swung = not swung place_heads() end
end

function redraw()
  screen.clear()
  screen.level(2)
  screen.move(4, 36)
  screen.line(124, 36)
  screen.stroke()
  local maxt = times[4] or 1
  for i = 1, 5 do
    local t = i == 1 and 0 or times[i - 1]
    local x = 6 + t / maxt * 112
    local h = i == 1 and 20 or 16 * params:get("falloff") ^ (i - 2)
    screen.level(math.max(3, lit[i]))
    screen.rect(x, 36 - h, 4, h)
    screen.fill()
    if i > 1 then
      screen.level(5)
      screen.pixel(64 + PANS[i - 1] * 50, 44)
      screen.fill()
    end
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text(swung and "multitap (swung)" or "multitap")
  screen.level(4)
  screen.move(0, 62)
  screen.text("space " .. params:string("spacing"))
  screen.move(127, 62)
  screen.text_right(params:string("tone"))
  screen.update()
end

function cleanup()
  softcut.rec(1, 0)
end
