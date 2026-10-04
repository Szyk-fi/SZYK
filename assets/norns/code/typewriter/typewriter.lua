-- typewriter
-- a Portamax norns script
--
-- an old typewriter taps out a short
-- text, key by key, at a human pace.
-- every letter is a note; spaces are
-- rests; full stops drop low. at the
-- end of each line the bell dings
-- and the carriage return plays a chord.
--
-- E2 words per minute   E3 tone
-- K2 next paragraph   K3 pause
-- pads: strike a key
-- (params: scale, root, line width)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local TEXT = {
  "the kettle hums in the next room. rain is counting the window panes.",
  "nobody will read this letter, so it can say whatever it likes.",
  "the cat has opinions about the radiator, and so do i.",
  "tomorrow the ribbon runs out. today there is still ink.",
}
local scale = {}
local para, pos = 1, 0
local lines = { "" }
local paused = false
local hammer = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 16)
end

local function hz(n) engine.hz(MusicUtil.note_num_to_freq(n)) end

local function strike(ch)
  local b = string.byte(ch)
  if ch == " " then return end
  engine.release(0.25)
  if ch == "." or ch == "," then
    engine.amp(0.25) engine.pw(0.1)
    hz(scale[1] - 12)
  elseif b >= 97 and b <= 122 then
    -- vowels ring a little longer and louder
    local vowel = ch:find("[aeiou]") ~= nil
    engine.amp(vowel and 0.22 or 0.15)
    engine.release(vowel and 0.6 or 0.2)
    engine.pw(0.3)
    hz(scale[(b - 97) % #scale + 1])
  end
  hammer = 3
end

local function carriage_return()
  engine.release(1.6) engine.amp(0.12) engine.pw(0.5)
  hz(scale[1] + 24) -- the bell
  local c = MusicUtil.generate_chord(scale[1] - 12, "major 7", 0)
  for _, n in ipairs(c) do hz(n) end
  table.insert(lines, "")
  if #lines > 5 then table.remove(lines, 1) end
end

local function type_char(ch)
  local width = params:get("width")
  if ch == " " and #lines[#lines] >= width - 5 then carriage_return() return end
  if #lines[#lines] >= width then carriage_return() end
  lines[#lines] = lines[#lines] .. ch
  strike(ch)
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("TYPEWRITER")
  params:add_option("scale", "scale", names, 11)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 48, 72, 60, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_number("wpm", "words per minute", 15, 120, 55)
  params:add_number("width", "line width", 12, 21, 20)
  params:add_control("tone", "tone", controlspec.new(500, 6000, 'exp', 0, 2400, 'hz'))
  params:set_action("tone", function(x) engine.cutoff(x) end)
  params:default()
  math.randomseed(os.time())
  build_scale()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then type_char(string.char(97 + (msg.note - 60) % 26)) end
  end
  clock.run(function()
    while true do
      local wait = 60 / (params:get("wpm") * 5)
      if not paused then
        pos = pos + 1
        local t = TEXT[para]
        if pos > #t then
          carriage_return()
          para, pos = para % #TEXT + 1, 0
          wait = wait * 6
        else
          local ch = t:sub(pos, pos)
          type_char(ch)
          if ch == "." then wait = wait * 3 end
        end
      end
      -- a person types unevenly
      clock.sleep(wait * (0.7 + math.random() * 0.6))
    end
  end)
  clock.run(function()
    while true do
      clock.sleep(1 / 20)
      hammer = math.max(0, hammer - 1)
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("wpm", d)
  elseif n == 3 then params:delta("tone", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then carriage_return() para, pos = para % #TEXT + 1, 0
  elseif n == 3 then paused = not paused end
  redraw()
end

function redraw()
  screen.clear()
  screen.level(2)
  screen.rect(2, 12, 124, 42)
  screen.fill()
  for i, l in ipairs(lines) do
    screen.level(i == #lines and 15 or 6 + i)
    screen.move(4, 12 + i * 8)
    screen.text(l)
  end
  -- the carriage and the type bar
  local cx = 4 + #lines[#lines] * 5
  screen.level(hammer > 0 and 15 or 4)
  screen.move(cx, 54) screen.line(cx, 58 - hammer) screen.stroke()
  screen.level(15)
  screen.move(0, 8)
  screen.text(paused and "typewriter (paused)" or "typewriter")
  screen.level(4)
  screen.move(0, 62)
  screen.text(params:get("wpm") .. " wpm")
  screen.move(127, 62)
  screen.text_right("page " .. para)
  screen.update()
end
