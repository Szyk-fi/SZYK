-- reverso
-- a Portamax norns script
--
-- a slow melody, and behind it its
-- own shadow played backwards. one
-- softcut head writes two bars of
-- tape; a second head reads the
-- same tape at a negative rate, so
-- every phrase swells in reverse.
--
-- E2 echo level   E3 reverse speed
-- K2 new melody   K3 echo side swap
-- (params: root, tone, length)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local RATES = { -0.5, -1, -2 }
local melody = {}
local idx = 0
local side = 1
local wpos, rpos = 0, 0
local trail = {}

local function new_melody()
  local s = MusicUtil.generate_scale_of_length(params:get("root"), "Natural Minor", 12)
  melody = {}
  local d = 5
  for i = 1, 8 do
    d = util.clamp(d + math.random(-2, 2), 1, #s)
    melody[i] = (i % 4 == 0 and math.random() < 0.5) and 0 or s[d]
  end
end

local function setup()
  local len = params:get("length")
  audio.level_eng_cut(1)
  softcut.buffer_clear()
  for v = 1, 2 do
    softcut.enable(v, 1)
    softcut.buffer(v, 1)
    softcut.loop(v, 1)
    softcut.loop_start(v, 1)
    softcut.loop_end(v, 1 + len)
    softcut.fade_time(v, 0.05)
    softcut.play(v, 1)
  end
  softcut.position(1, 1)
  softcut.position(2, 1 + len)
  softcut.level(1, 0)
  softcut.level_input_cut(1, 1, 1.0)
  softcut.level_input_cut(2, 1, 1.0)
  softcut.rec_level(1, 1)
  softcut.pre_level(1, 0.25)
  softcut.rec(1, 1)
  softcut.rec(2, 0)
  softcut.rate(2, RATES[params:get("rate")])
  softcut.level(2, params:get("echo"))
  softcut.pan(2, 0.6 * side)
  softcut.filter_dry(2, 0.4)
  softcut.filter_lp(2, 0.6)
  softcut.filter_fc(2, 1800)
end

function init()
  params:add_separator("REVERSO")
  params:add_number("root", "root", 48, 67, 57, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", new_melody)
  params:add_control("echo", "echo level", controlspec.new(0, 1.2, 'lin', 0, 0.8, ''))
  params:set_action("echo", function(x) softcut.level(2, x) end)
  params:add_option("rate", "reverse speed", { "-0.5x", "-1x", "-2x" }, 2)
  params:set_action("rate", function(i) softcut.rate(2, RATES[i]) end)
  params:add_control("length", "tape length", controlspec.new(1, 6, 'lin', 0.5, 4, 's'))
  params:set_action("length", function(x) softcut.loop_end(1, 1 + x) softcut.loop_end(2, 1 + x) end)
  params:add_control("tone", "tone", controlspec.new(400, 5000, 'exp', 0, 1500, 'hz'))
  params:set_action("tone", function(x) engine.cutoff(x) end)
  params:default()
  engine.release(1.4)
  engine.amp(0.26)
  engine.pw(0.3)
  engine.pan(-0.3)
  math.randomseed(os.time())
  new_melody()
  setup()
  clock.run(function()
    while true do
      clock.sync(1 / 2)
      idx = idx % #melody + 1
      local n = melody[idx]
      if n > 0 then
        engine.pan(-0.4 * side)
        engine.hz(MusicUtil.note_num_to_freq(n))
        table.insert(trail, { x = wpos, l = 15 })
      end
    end
  end)
  clock.run(function()
    while true do
      clock.sleep(1 / 15)
      local len = params:get("length")
      wpos = (wpos + 1 / 15 / len) % 1
      rpos = (rpos + RATES[params:get("rate")] / 15 / len) % 1
      for i = #trail, 1, -1 do
        trail[i].l = trail[i].l - 0.25
        if trail[i].l < 1 then table.remove(trail, i) end
      end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("echo", d)
  elseif n == 3 then params:delta("rate", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_melody()
  elseif n == 3 then
    side = -side
    softcut.pan(2, 0.6 * side)
  end
end

function redraw()
  screen.clear()
  screen.level(2)
  screen.move(4, 34)
  screen.line(124, 34)
  screen.stroke()
  for _, t in ipairs(trail) do
    screen.level(math.floor(t.l))
    screen.rect(4 + t.x * 118, 28, 2, 4)
    screen.fill()
  end
  screen.level(15)
  screen.rect(4 + wpos * 118, 24, 1, 10)
  screen.fill()
  screen.level(8)
  screen.rect(4 + rpos * 118, 34, 1, 10)
  screen.fill()
  screen.move(4 + rpos * 118 - 3, 52)
  screen.text("<")
  screen.level(15)
  screen.move(0, 8)
  screen.text("reverso")
  screen.level(4)
  screen.move(0, 62)
  screen.text("echo " .. string.format("%.2f", params:get("echo")))
  screen.move(127, 62)
  screen.text_right(params:string("rate") .. (side > 0 and " R" or " L"))
  screen.update()
end

function cleanup()
  softcut.rec(1, 0)
end
