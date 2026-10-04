-- pingpong
-- a Portamax norns script
--
-- short plucks thrown into a
-- ping-pong delay. the left head
-- echoes and feeds itself; the
-- right head reads the same tape
-- half a loop later, so each echo
-- bounces across before it returns.
--
-- E2 delay time   E3 feedback
-- K2 new motif   K3 swap sides
-- (params: tone, width)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local motif = {}
local mstep = 0
local flip = 1
local hits = {}

local function dtime() return params:get("time") * 60 / clock.get_tempo() end

local function place()
  local t = dtime()
  for v = 1, 2 do
    softcut.loop_start(v, 1)
    softcut.loop_end(v, 1 + t)
  end
  softcut.position(1, 1)
  softcut.position(2, 1 + t / 2)
end

local function pans()
  local w = params:get("width")
  softcut.pan(1, -w * flip)
  softcut.pan(2, w * flip)
end

local function setup()
  audio.level_eng_cut(1)
  softcut.buffer_clear()
  for v = 1, 2 do
    softcut.enable(v, 1)
    softcut.buffer(v, 1)
    softcut.loop(v, 1)
    softcut.rate(v, 1)
    softcut.fade_time(v, 0.01)
    softcut.play(v, 1)
    softcut.level(v, 0.75)
    softcut.filter_dry(v, 0)
    softcut.filter_lp(v, 1)
    softcut.filter_rq(v, 1.4)
  end
  softcut.level_input_cut(1, 1, 1.0)
  softcut.level_input_cut(2, 1, 1.0)
  softcut.rec_level(1, 1)
  softcut.rec(1, 1)
  softcut.rec(2, 0)
end

local function new_motif()
  local s = MusicUtil.generate_scale_of_length(67, "Major Pentatonic", 8)
  motif = {}
  for i = 1, 16 do motif[i] = (i % 8 == 1 or math.random() < 0.18) and s[math.random(#s)] or 0 end
end

function init()
  params:add_separator("PINGPONG")
  setup()
  params:add_control("time", "delay time", controlspec.new(0.25, 2, 'lin', 0.25, 0.75, 'beats'))
  params:set_action("time", function() place() end)
  params:add_control("fb", "feedback", controlspec.new(0, 0.9, 'lin', 0, 0.6, ''))
  params:set_action("fb", function(x) softcut.pre_level(1, x) end)
  params:add_control("tone", "echo tone", controlspec.new(500, 8000, 'exp', 0, 2800, 'hz'))
  params:set_action("tone", function(x) softcut.filter_fc(1, x) softcut.filter_fc(2, x * 0.8) end)
  params:add_control("width", "width", controlspec.new(0, 1, 'lin', 0, 0.9, ''))
  params:set_action("width", pans)
  params:default()
  engine.release(0.35)
  engine.amp(0.3)
  engine.cutoff(3500)
  engine.pw(0.5)
  math.randomseed(os.time())
  new_motif()
  clock.run(function()
    while true do
      clock.sync(1 / 4)
      mstep = mstep % #motif + 1
      local n = motif[mstep]
      if n > 0 then
        engine.hz(MusicUtil.note_num_to_freq(n))
        table.insert(hits, { age = 0 })
      end
    end
  end)
  clock.run(function()
    while true do
      clock.sleep(1 / 20)
      for i = #hits, 1, -1 do
        hits[i].age = hits[i].age + 1 / 20
        if hits[i].age > dtime() * 6 then table.remove(hits, i) end
      end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("time", d)
  elseif n == 3 then params:delta("fb", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_motif()
  elseif n == 3 then flip = -flip pans() end
end

function redraw()
  screen.clear()
  screen.level(3)
  screen.rect(2, 16, 3, 36)
  screen.rect(123, 16, 3, 36)
  screen.fill()
  local half = dtime() / 2
  local fb = params:get("fb")
  for _, h in ipairs(hits) do
    -- the ball reaches the right wall at t/2, the left at t, ...
    local leg = h.age / half
    local k = math.floor(leg)
    local f = leg - k
    local x
    if k == 0 then x = 64 + 59 * flip * f
    elseif k % 2 == 1 then x = 64 + 59 * flip * (1 - 2 * f)
    else x = 64 + 59 * flip * (2 * f - 1) end
    local y = 50 - math.sin(f * math.pi) * 22
    screen.level(math.max(1, math.floor(15 * fb ^ (k / 2))))
    screen.circle(x, y, 2)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text("pingpong")
  screen.level(4)
  screen.move(0, 62)
  screen.text("time " .. params:string("time"))
  screen.move(127, 62)
  screen.text_right("fb " .. string.format("%.2f", fb))
  screen.update()
end

function cleanup()
  softcut.rec(1, 0)
end
