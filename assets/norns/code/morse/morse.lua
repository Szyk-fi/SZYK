-- morse
-- a Portamax norns script
--
-- a shortwave key sending messages
-- in morse code. dots and dashes ride
-- a tone that steps to a new pitch on
-- each word, while a slow bass walks
-- underneath. the tape shows the code.
--
-- E2 words per minute   E3 tone pitch
-- K2 next message   K3 pause
-- pads: tap the key
-- (params: root, bass level)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local CODE = {
  a = ".-", b = "-...", c = "-.-.", d = "-..", e = ".", f = "..-.", g = "--.", h = "....",
  i = "..", j = ".---", k = "-.-", l = ".-..", m = "--", n = "-.", o = "---", p = ".--.",
  q = "--.-", r = ".-.", s = "...", t = "-", u = "..-", v = "...-", w = ".--", x = "-..-",
  y = "-.--", z = "--..",
}
local MSGS = { "calm seas tonight", "lamps lit all well", "fog lifting by dawn", "wind west bring tea" }
local BASS = { 0, -5, -2, -7 }
local scale = {}
local msg, ci, word = 1, 0, 0
local tape = {}
local key_down = 0
local paused = false

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), "Major Pentatonic", 12)
end

local function unit() return 1.2 / params:get("wpm") end

local function beep(len)
  local n = scale[util.clamp(params:get("pitch") + word % 3, 1, #scale)] + 12
  engine.release(len * unit() * 1.5)
  engine.amp(0.2) engine.pw(0.5) engine.cutoff(2600) engine.pan(0.2)
  engine.hz(MusicUtil.note_num_to_freq(n))
  table.insert(tape, len)
  if #tape > 40 then table.remove(tape, 1) end
  key_down = math.max(2, math.floor(len * unit() * 20))
end

local function gap(units)
  table.insert(tape, -units)
  if #tape > 40 then table.remove(tape, 1) end
  clock.sleep(units * unit())
end

function init()
  params:add_separator("MORSE")
  params:add_number("root", "root", 48, 67, 57, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_number("wpm", "words per minute", 6, 30, 14)
  params:add_number("pitch", "tone pitch", 1, 9, 5)
  params:add_control("bass", "bass level", controlspec.new(0, 0.4, 'lin', 0, 0.25, ''))
  params:default()
  build_scale()
  local m = midi.connect()
  m.event = function(data)
    local d = midi.to_msg(data)
    if d.type == "note_on" then beep(d.note % 2 == 0 and 1 or 3) end
  end
  clock.run(function()
    while true do
      if paused then clock.sleep(0.1) else
        ci = ci + 1
        local text = MSGS[msg]
        if ci > #text then
          ci, msg, word = 0, msg % #MSGS + 1, 0
          gap(14)
        else
          local ch = text:sub(ci, ci)
          if ch == " " then word = word + 1 gap(7) else
            local code = CODE[ch] or ""
            for k = 1, #code do
              beep(code:sub(k, k) == "." and 1 or 3)
              clock.sleep((code:sub(k, k) == "." and 1 or 3) * unit())
              if k < #code then gap(1) end
            end
            gap(3)
          end
        end
      end
    end
  end)
  -- the slow walking bass, one note every eight beats of code
  clock.run(function()
    local i = 0
    while true do
      if not paused and params:get("bass") > 0 then
        i = i + 1
        engine.release(unit() * 12) engine.amp(params:get("bass"))
        engine.pw(0.15) engine.cutoff(700) engine.pan(-0.2)
        engine.hz(MusicUtil.note_num_to_freq(params:get("root") - 24 + BASS[(i - 1) % #BASS + 1]))
      end
      clock.sleep(unit() * 8)
    end
  end)
  clock.run(function()
    while true do
      clock.sleep(1 / 20)
      key_down = math.max(0, key_down - 1)
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("wpm", d)
  elseif n == 3 then params:delta("pitch", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then msg, ci, word = msg % #MSGS + 1, 0, 0 beep(3)
  elseif n == 3 then paused = not paused end
  redraw()
end

function redraw()
  screen.clear()
  local text = MSGS[msg]
  local sent = text:sub(1, math.max(0, ci - 1))
  local w = screen.text_extents(sent)
  screen.level(4)
  screen.move(0, 22)
  screen.text(sent)
  screen.level(15)
  screen.move(w, 22)
  screen.text(text:sub(ci, ci))
  screen.level(2)
  screen.move(w + screen.text_extents(text:sub(ci, ci)), 22)
  screen.text(text:sub(ci + 1))
  -- paper tape scrolling left, newest mark at the right
  local x = 124
  for i = #tape, 1, -1 do
    local t = tape[i]
    if t > 0 then
      x = x - t * 2
      screen.level(12)
      screen.rect(x, 38, t * 2, 3)
      screen.fill()
    else
      x = x + t * 2
    end
    if x < 0 then break end
  end
  screen.level(2)
  screen.move(0, 34) screen.line(128, 34) screen.stroke()
  screen.move(0, 45) screen.line(128, 45) screen.stroke()
  screen.level(key_down > 0 and 15 or 3)
  screen.circle(122, 5, 3)
  screen.fill()
  screen.level(15)
  screen.move(0, 8)
  screen.text(paused and "morse (paused)" or "morse")
  screen.level(4)
  screen.move(0, 62)
  screen.text(params:get("wpm") .. " wpm")
  screen.move(127, 62)
  screen.text_right(MusicUtil.note_num_to_name(scale[util.clamp(params:get("pitch"), 1, #scale)], true))
  screen.update()
end
