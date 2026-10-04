-- looper
-- a Portamax norns script
--
-- play the pads; every note goes
-- into a four-bar softcut loop.
-- overdub layer on layer, then
-- slow it, reverse it, or let it
-- fade.
--
-- E2 loop speed   E3 fade
-- K2 record on/off   K3 clear
-- (params: tone)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local recording = true
local LOOP = 8
local played = {}
local pos = 0

local function setup()
  audio.level_eng_cut(1)
  softcut.buffer_clear()
  softcut.enable(1, 1)
  softcut.buffer(1, 1)
  softcut.level(1, 0.8)
  softcut.pan(1, 0)
  softcut.rate(1, 1)
  softcut.loop(1, 1)
  softcut.loop_start(1, 1)
  softcut.loop_end(1, 1 + LOOP)
  softcut.position(1, 1)
  softcut.fade_time(1, 0.02)
  softcut.level_input_cut(1, 1, 1.0)
  softcut.level_input_cut(2, 1, 1.0)
  softcut.rec_level(1, 1)
  softcut.pre_level(1, 0.9)
  softcut.play(1, 1)
  softcut.rec(1, 1)
  softcut.rate_slew_time(1, 0.4)
end

function init()
  params:add_separator("LOOPER")
  params:add_control("tone", "tone", controlspec.new(300, 6000, 'exp', 0, 1800, 'hz'))
  params:set_action("tone", function(x) engine.cutoff(x) end)
  params:add_control("speed", "loop speed", controlspec.new(-2, 2, 'lin', 0.25, 1, 'x'))
  params:set_action("speed", function(x) softcut.rate(1, x) end)
  params:add_control("fade", "fade", controlspec.new(0, 1, 'lin', 0, 0.9, ''))
  params:set_action("fade", function(x) softcut.pre_level(1, x) end)
  params:default()
  engine.release(1.2)
  engine.amp(0.3)
  setup()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      engine.hz(MusicUtil.note_num_to_freq(msg.note))
      table.insert(played, { at = pos, note = msg.note })
      while #played > 64 do table.remove(played, 1) end
    end
  end
  -- with nothing played yet, a few starter notes so the loop isn't silent
  clock.run(function()
    for _, n in ipairs({ 60, 64, 67, 71 }) do
      clock.sync(1)
      if #played < 4 then
        engine.hz(MusicUtil.note_num_to_freq(n))
        table.insert(played, { at = pos, note = n })
      end
    end
  end)
  local tick = metro.init(function()
    pos = (pos + params:get("speed") / 30) % LOOP
    redraw()
  end, 1 / 30)
  tick:start()
end

function enc(n, d)
  if n == 2 then params:delta("speed", d)
  elseif n == 3 then params:delta("fade", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    recording = not recording
    softcut.rec(1, recording and 1 or 0)
  elseif n == 3 then
    softcut.buffer_clear()
    played = {}
  end
end

function redraw()
  screen.clear()
  screen.level(3)
  screen.rect(4, 30, 120, 2)
  screen.fill()
  for _, p in ipairs(played) do
    screen.level(8)
    screen.rect(4 + p.at / LOOP * 120, 50 - (p.note - 48) * 0.8, 2, 2)
    screen.fill()
  end
  screen.level(15)
  screen.rect(4 + pos / LOOP * 120, 26, 1, 10)
  screen.fill()
  screen.move(0, 7)
  screen.text(recording and "looper (rec)" or "looper")
  screen.level(4)
  screen.move(0, 62)
  screen.text("speed " .. params:string("speed"))
  screen.move(127, 62)
  screen.text_right("pads play")
  screen.update()
end

function cleanup()
  softcut.rec(1, 0)
end
