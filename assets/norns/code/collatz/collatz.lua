-- collatz
-- a Portamax norns script
--
-- pick a number. if it is even,
-- halve it; if odd, triple it and
-- add one. every start seems to
-- fall to 1 in the end. each value
-- on the way down is a note: odd
-- steps leap and ring, even steps
-- fall softly.
--
-- E2 start number   E3 scale
-- K2 next number   K3 pause
-- pads: root note
-- (params: tempo division, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local DIVS = { 1 / 4, 1 / 3, 1 / 2 }
local DIV_NAMES = { "1/16", "1/8t", "1/8" }
local seq = {}
local pos = 1
local scale = {}
local root = 48
local paused = false
local peak = 1

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(root, params:get("scale"), 21)
end

local function build_seq()
  seq = {}
  local n = params:get("start")
  seq[1] = n
  while n ~= 1 and #seq < 400 do
    if n % 2 == 0 then n = n // 2 else n = 3 * n + 1 end
    seq[#seq + 1] = n
  end
  peak = 1
  for _, v in ipairs(seq) do peak = math.max(peak, v) end
  pos = 1
end

local function play(v, prev)
  -- height (log of the value) picks the register, the value itself the degree
  local h = math.log(v, 2) / math.log(math.max(peak, 2), 2)
  local deg = math.floor(h * 12) + (v % 3) + 1
  local note = scale[util.clamp(deg, 1, #scale)]
  local odd = prev and prev % 2 == 1
  engine.amp(odd and 0.32 or 0.18)
  engine.pw(odd and 0.35 or 0.6)
  engine.pan(util.linlin(0, 1, -0.6, 0.6, h))
  engine.hz(MusicUtil.note_num_to_freq(note))
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("COLLATZ")
  params:add_number("start", "start number", 2, 9999, 27)
  params:set_action("start", build_seq)
  params:add_option("scale", "scale", names, 5)
  params:set_action("scale", build_scale)
  params:add_option("div", "step", DIV_NAMES, 3)
  params:add_control("release", "release", controlspec.new(0.1, 3, 'exp', 0, 0.9, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:add_control("cutoff", "cutoff", controlspec.new(300, 8000, 'exp', 0, 2200, 'hz'))
  params:set_action("cutoff", function(x) engine.cutoff(x) end)
  params:default()
  build_scale()
  build_seq()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then root = msg.note - 12 build_scale() end
  end
  clock.run(function()
    while true do
      if not paused then
        play(seq[pos], seq[pos - 1])
        pos = pos + 1
        if pos > #seq then
          -- land on 1, then move on to the next start
          clock.sync(1)
          params:set("start", params:get("start") >= 9999 and 2 or params:get("start") + 1)
        end
      end
      redraw()
      clock.sync(DIVS[params:get("div")])
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("start", d)
  elseif n == 3 then params:delta("scale", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then params:set("start", math.random(2, 999))
  elseif n == 3 then paused = not paused end
  redraw()
end

function redraw()
  screen.clear()
  screen.level(15)
  screen.move(0, 8)
  screen.text("collatz " .. params:get("start"))
  local w = math.max(1, 126 / #seq)
  local lp = math.log(math.max(peak, 2))
  for i, v in ipairs(seq) do
    local h = math.log(v) / lp * 40
    screen.level(i == pos - 1 and 15 or (i < pos and 6 or 2))
    screen.rect(1 + (i - 1) * w, 54 - h, math.max(1, w - 0.5), h + 1)
    screen.fill()
  end
  screen.level(4)
  screen.move(0, 62)
  screen.text(#seq - 1 .. " steps  ^" .. peak)
  screen.move(127, 62)
  local cur = seq[math.max(1, pos - 1)]
  screen.text_right(paused and "paused" or tostring(cur))
  screen.update()
end
