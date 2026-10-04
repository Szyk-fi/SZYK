-- tapeloop
-- a Portamax norns script
--
-- a short tape loop that never
-- stops recording. new notes are
-- laid over old ones, and each pass
-- the old ones lose a little more
-- (pre level < 1) while the motor
-- drifts in speed.
--
-- E2 wear   E3 drift
-- K2 splice (clear)   K3 feed on/off
-- (params: loop length, tone, density)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local scale = {}
local feeding = true
local reel = 0
local drift_phase = 0
local speed = 1
local marks = {}
local ticks = 0

local function setup_tape()
  local len = params:get("length")
  audio.level_eng_cut(1)
  softcut.buffer_clear()
  softcut.enable(1, 1)
  softcut.buffer(1, 1)
  softcut.level(1, 0.8)
  softcut.pan(1, 0)
  softcut.rate(1, 1)
  softcut.rate_slew_time(1, 0.6)
  softcut.loop(1, 1)
  softcut.loop_start(1, 1)
  softcut.loop_end(1, 1 + len)
  softcut.position(1, 1)
  softcut.fade_time(1, 0.05)
  softcut.level_input_cut(1, 1, 1.0)
  softcut.level_input_cut(2, 1, 1.0)
  softcut.rec_level(1, 0.8)
  softcut.pre_level(1, 1 - params:get("wear"))
  softcut.play(1, 1)
  softcut.rec(1, 1)
  softcut.filter_dry(1, 0.3)
  softcut.filter_lp(1, 0.7)
  softcut.filter_fc(1, 3000)
  softcut.filter_rq(1, 1.2)
end

function init()
  params:add_separator("TAPELOOP")
  params:add_control("length", "loop length", controlspec.new(2, 8, 'lin', 0.5, 4, 's'))
  params:set_action("length", function(x) softcut.loop_end(1, 1 + x) end)
  params:add_control("wear", "wear", controlspec.new(0.02, 0.5, 'lin', 0, 0.12, ''))
  params:set_action("wear", function(x) softcut.pre_level(1, 1 - x) end)
  params:add_control("drift", "drift", controlspec.new(0, 0.1, 'lin', 0, 0.025, ''))
  params:add_control("tone", "tone", controlspec.new(300, 4000, 'exp', 0, 1300, 'hz'))
  params:set_action("tone", function(x) engine.cutoff(x) end)
  params:add_number("density", "density", 1, 8, 3)
  params:default()
  engine.release(1.6)
  engine.amp(0.22)
  engine.pw(0.4)
  scale = MusicUtil.generate_scale_of_length(57, "Minor Pentatonic", 11)
  math.randomseed(os.time())
  setup_tape()
  clock.run(function()
    while true do
      clock.sync(1 / 2)
      ticks = ticks + 1
      -- every fourth half-beat always speaks, so the loop never runs dry
      if feeding and (ticks % 4 == 1 or math.random(8) <= params:get("density")) then
        local n = scale[math.random(#scale)]
        engine.pan(math.random() - 0.5)
        engine.hz(MusicUtil.note_num_to_freq(n))
        table.insert(marks, { a = reel, life = 15 })
      end
    end
  end)
  clock.run(function()
    while true do
      clock.sleep(1 / 15)
      -- slow wow from two unrelated wobbles: never quite repeats
      drift_phase = drift_phase + 1 / 15
      local d = params:get("drift")
      speed = 1 + d * (math.sin(drift_phase * 0.7) + 0.5 * math.sin(drift_phase * 2.3))
      softcut.rate(1, speed)
      reel = (reel + speed / 15 / params:get("length")) % 1
      for i = #marks, 1, -1 do
        marks[i].life = marks[i].life - 0.02 * (1 + params:get("wear") * 10)
        if marks[i].life <= 0 then table.remove(marks, i) end
      end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("wear", d)
  elseif n == 3 then params:delta("drift", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then softcut.buffer_clear() marks = {}
  elseif n == 3 then feeding = not feeding end
end

local function reel_at(cx, r)
  screen.level(6)
  screen.circle(cx, 32, r)
  screen.stroke()
  for k = 0, 2 do
    local a = reel * 2 * math.pi + k * 2.094
    screen.move(cx, 32)
    screen.line(cx + math.cos(a) * r, 32 + math.sin(a) * r)
    screen.stroke()
  end
end

function redraw()
  screen.clear()
  reel_at(34, 14)
  reel_at(94, 14)
  screen.level(3)
  screen.move(34, 46)
  screen.line(94, 46)
  screen.stroke()
  for _, m in ipairs(marks) do
    local x = 34 + ((reel - m.a) % 1) * 60
    screen.level(math.max(1, math.floor(m.life)))
    screen.rect(x, 44, 2, 4)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text(feeding and "tapeloop" or "tapeloop (no feed)")
  screen.level(4)
  screen.move(0, 62)
  screen.text("wear " .. string.format("%.2f", params:get("wear")))
  screen.move(127, 62)
  screen.text_right(string.format("x%.3f", speed))
  screen.update()
end

function cleanup()
  softcut.rec(1, 0)
end
