-- graycode
-- a Portamax norns script
--
-- counting in gray code: from one
-- number to the next only a single
-- bit ever changes. each bit owns a
-- note of a chord; when it flips on
-- it sounds bright, when it flips
-- off it sounds soft and low. the
-- low bit flips most, the top bit
-- almost never.
--
-- E2 bits   E3 chord
-- K2 reset count   K3 pause
-- pads: root note
-- (params: speed, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local CHORDS = {
  { "maj9", { 0, 4, 7, 11, 14, 16, 19, 23 } },
  { "min9", { 0, 3, 7, 10, 14, 15, 19, 22 } },
  { "sus", { 0, 5, 7, 12, 14, 17, 19, 24 } },
  { "lydian", { 0, 7, 11, 14, 18, 19, 23, 26 } },
}
local n = 0
local code = 0
local flipped = 0
local history = {}
local root = 48
local paused = false

local function gray(i) return i ~ (i >> 1) end

local function tick()
  local bits = params:get("bits")
  n = (n + 1) % (1 << bits)
  local g = gray(n)
  local diff = g ~ code
  code = g
  -- exactly one bit differs; find which
  flipped = 0
  while diff > 1 do diff = diff >> 1 flipped = flipped + 1 end
  local on = (code >> flipped) & 1 == 1
  local iv = CHORDS[params:get("chord")][2][flipped + 1]
  local note = root + iv + (on and 12 or 0)
  engine.amp(on and 0.26 or 0.16)
  engine.pw(on and 0.3 or 0.6)
  engine.pan(util.linlin(0, bits - 1, 0.6, -0.6, flipped))
  engine.hz(MusicUtil.note_num_to_freq(note))
  table.insert(history, 1, code)
  if #history > 40 then table.remove(history) end
end

function init()
  local cnames = {}
  for i, c in ipairs(CHORDS) do cnames[i] = c[1] end
  params:add_separator("GRAYCODE")
  params:add_number("bits", "bits", 3, 8, 5)
  params:set_action("bits", function(b) n = n % (1 << b) code = gray(n) end)
  params:add_option("chord", "chord", cnames, 1)
  params:add_option("speed", "speed", { "1/16", "1/8t", "1/8" }, 2)
  params:add_control("release", "release", controlspec.new(0.1, 3, 'exp', 0, 1.0, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:add_control("cutoff", "cutoff", controlspec.new(300, 8000, 'exp', 0, 2400, 'hz'))
  params:set_action("cutoff", function(x) engine.cutoff(x) end)
  params:default()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then root = msg.note - 12 end
  end
  local divs = { 1 / 4, 1 / 3, 1 / 2 }
  clock.run(function()
    while true do
      if not paused then tick() end
      redraw()
      clock.sync(divs[params:get("speed")])
    end
  end)
end

function enc(e, d)
  if e == 2 then params:delta("bits", d)
  elseif e == 3 then params:delta("chord", d) end
  redraw()
end

function key(k, z)
  if z == 0 then return end
  if k == 2 then n = 0 code = 0 history = {}
  elseif k == 3 then paused = not paused end
  redraw()
end

function redraw()
  screen.clear()
  local bits = params:get("bits")
  local w = math.floor(120 / bits)
  -- the current code word, high bit on the left
  for b = 0, bits - 1 do
    local x = 4 + (bits - 1 - b) * w
    local on = (code >> b) & 1 == 1
    screen.level(b == flipped and 15 or (on and 8 or 2))
    screen.rect(x, 13, w - 3, 10)
    if on then screen.fill() else screen.stroke() end
  end
  -- recent history scrolls down: each row one count
  for i, c in ipairs(history) do
    if i > 14 then break end
    for b = 0, bits - 1 do
      if (c >> b) & 1 == 1 then
        screen.level(math.max(1, 9 - i // 2))
        screen.rect(4 + (bits - 1 - b) * w, 25 + i * 2, w - 3, 1)
        screen.fill()
      end
    end
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text("graycode")
  screen.level(4)
  screen.move(127, 8)
  screen.text_right(n .. " / " .. (1 << bits))
  screen.move(0, 62)
  screen.text(CHORDS[params:get("chord")][1] .. "  bit " .. flipped)
  screen.move(127, 62)
  screen.text_right(paused and "paused" or "")
  screen.update()
end
