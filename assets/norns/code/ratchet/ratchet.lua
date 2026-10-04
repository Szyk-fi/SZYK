-- ratchet
-- a Portamax norns script
--
-- an eight-step sequencer where any
-- step can fire as a ratchet: two,
-- three or four rapid repeats
-- squeezed into the step's time,
-- rising or falling in loudness.
--
-- E2 select step   E3 ratchets
-- K2 new sequence   K3 ramp up / down
-- pads: set the selected step's note
-- (params: step length, tone)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local steps = {}
local pos = 0
local sel = 1
local ramp_up = false
local sub = 0

local function randomise()
  local s = MusicUtil.generate_scale_of_length(48, "Harmonic Minor", 10)
  for i = 1, 8 do
    steps[i] = { note = s[math.random(#s)], r = math.random() < 0.35 and math.random(2, 4) or 1 }
  end
  steps[1].note = s[1]
end

function init()
  params:add_separator("RATCHET")
  params:add_option("len", "step length", { "1/8", "1/4", "1/2" }, 2)
  params:add_control("tone", "tone", controlspec.new(300, 5000, 'exp', 0, 1400, 'hz'))
  params:set_action("tone", function(x) engine.cutoff(x) end)
  params:add_control("decay", "decay", controlspec.new(0.05, 0.8, 'lin', 0, 0.2, 's'))
  params:set_action("decay", function(x) engine.release(x) end)
  params:default()
  engine.pw(0.3)
  math.randomseed(os.time())
  randomise()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then steps[sel].note = msg.note - 12 end
  end
  clock.run(function()
    while true do
      local beats = ({ 0.5, 1, 2 })[params:get("len")] / 2
      clock.sync(beats)
      pos = pos % 8 + 1
      local s = steps[pos]
      local gap = clock.get_beat_sec() * beats / s.r
      for k = 1, s.r do
        sub = k
        local f = s.r == 1 and 1 or (k - 1) / (s.r - 1)
        if not ramp_up then f = 1 - f end
        engine.amp(0.14 + 0.16 * f)
        engine.hz(MusicUtil.note_num_to_freq(s.note))
        redraw()
        if k < s.r then clock.sleep(gap) end
      end
    end
  end)
  clock.run(function()
    while true do
      clock.sleep(1 / 10)
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then sel = util.clamp(sel + d, 1, 8)
  elseif n == 3 then steps[sel].r = util.clamp(steps[sel].r + d, 1, 4) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then randomise()
  elseif n == 3 then ramp_up = not ramp_up end
  redraw()
end

function redraw()
  screen.clear()
  for i = 1, 8 do
    local s = steps[i]
    local x = 4 + (i - 1) * 15
    local y = 46 - (s.note - 44) * 1.2
    for k = 1, s.r do
      local f = s.r == 1 and 1 or (k - 1) / (s.r - 1)
      if not ramp_up then f = 1 - f end
      local lit = (i == pos and k <= sub)
      screen.level(lit and 15 or (i == sel and 7 or 3))
      local w = 12 / s.r
      local h = 2 + math.floor(f * 4)
      screen.rect(x + (k - 1) * w, y - h, math.max(1, w - 1), h)
      screen.fill()
    end
    screen.level(i == sel and 12 or 2)
    screen.rect(x, 52, 12, 1)
    screen.fill()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text("ratchet")
  screen.level(4)
  screen.move(0, 62)
  screen.text("step " .. sel .. " x" .. steps[sel].r)
  screen.move(127, 62)
  screen.text_right(ramp_up and "ramp up" or "ramp down")
  screen.update()
end
