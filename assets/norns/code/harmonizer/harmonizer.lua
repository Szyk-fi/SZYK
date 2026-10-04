-- harmonizer
-- a Portamax norns script
--
-- play a line on the pads (or MIDI
-- in) and it gains diatonic voices:
-- a third, a fifth or a sixth found
-- inside the chosen key, so the
-- harmony bends between major and
-- minor intervals as it should.
-- left alone, it sings a tune.
--
-- E2 harmony   E3 key
-- K2 new tune   K3 voice below / above
-- (params: mode, tone)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local HARM = {
  { name = "3rd", d = { 2 } },
  { name = "5th", d = { 4 } },
  { name = "6th", d = { 5 } },
  { name = "3rd + 5th", d = { 2, 4 } },
  { name = "3rd + 6th", d = { 2, 5 } },
  { name = "10th", d = { 9 } },
}
local MODES = { "Major", "Natural Minor", "Dorian", "Mixolydian" }
local scale = {}
local tune = {}
local tstep = 0
local below = false
local last_in = -10
local shown = {}

local function build()
  scale = MusicUtil.generate_scale_of_length(params:get("key") - 24, MODES[params:get("mode")], 36)
end

local function index_of(n)
  local s = MusicUtil.snap_note_to_array(n, scale)
  return tab.key(scale, s), s
end

local function harmonize(n, amp)
  local i, s = index_of(n)
  shown = { { n = s, lead = true } }
  engine.amp(amp)
  engine.pan(0)
  engine.hz(MusicUtil.note_num_to_freq(s))
  for k, d in ipairs(HARM[params:get("harm")].d) do
    local j = util.clamp(i + (below and -d or d), 1, #scale)
    engine.amp(amp * 0.7)
    engine.pan(k == 1 and -0.5 or 0.5)
    engine.hz(MusicUtil.note_num_to_freq(scale[j]))
    shown[#shown + 1] = { n = scale[j] }
  end
end

local function new_tune()
  tune = {}
  local deg = 0
  for i = 1, 8 do
    deg = util.clamp(deg + ({ -2, -1, -1, 0, 1, 1, 2 })[math.random(7)], -2, 6)
    tune[i] = (i == 4 or i == 8) and -99 or deg
  end
end

function init()
  params:add_separator("HARMONIZER")
  params:add_option("harm", "harmony", (function() local t = {} for i, h in ipairs(HARM) do t[i] = h.name end return t end)(), 1)
  params:add_number("key", "key", 55, 66, 60, function(p) return MusicUtil.note_num_to_name(p:get()) end)
  params:set_action("key", build)
  params:add_option("mode", "mode", MODES, 1)
  params:set_action("mode", build)
  params:add_control("tone", "tone", controlspec.new(400, 5000, 'exp', 0, 1700, 'hz'))
  params:set_action("tone", function(x) engine.cutoff(x) end)
  params:default()
  engine.release(1.2)
  engine.pw(0.4)
  math.randomseed(os.time())
  build()
  new_tune()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      last_in = util.time()
      harmonize(msg.note, 0.15 + msg.vel / 127 * 0.12)
    end
  end
  clock.run(function()
    while true do
      clock.sync(1 / 2)
      tstep = tstep % #tune + 1
      -- the tune only sings when nobody has played for a moment
      if util.time() - last_in > 1.5 and tune[tstep] > -99 then
        local i = index_of(params:get("key"))
        harmonize(scale[i + tune[tstep]], 0.2)
      end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("harm", d)
  elseif n == 3 then params:delta("key", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_tune()
  elseif n == 3 then below = not below end
  redraw()
end

function redraw()
  screen.clear()
  -- a little staff: each line a scale step, dots for the voices
  local base = index_of(params:get("key"))
  for l = 0, 4 do
    screen.level(2)
    screen.move(10, 20 + l * 7)
    screen.line(118, 20 + l * 7)
    screen.stroke()
  end
  for k, v in ipairs(shown) do
    local i = index_of(v.n)
    local y = 48 - (i - base + 4) * 3.5
    screen.level(v.lead and 15 or 8)
    screen.circle(40 + k * 18, util.clamp(y, 12, 54), v.lead and 3 or 2.5)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text("harmonizer")
  screen.move(127, 8)
  screen.text_right(MusicUtil.note_num_to_name(params:get("key")) .. " " .. ({ "maj", "min", "dor", "mix" })[params:get("mode")])
  screen.level(4)
  screen.move(0, 62)
  screen.text(HARM[params:get("harm")].name .. (below and " below" or " above"))
  screen.move(127, 62)
  screen.text_right(util.time() - last_in > 1.5 and "tune" or "pads")
  screen.update()
end
