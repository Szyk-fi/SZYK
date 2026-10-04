-- primes
-- a Portamax norns script
--
-- walks up the prime numbers. the
-- gap to the next prime is how long
-- each note lasts, so twin primes
-- tumble and the lonely stretches
-- hang in the air. each prime's
-- remainder picks the scale degree.
--
-- E2 jump through primes   E3 unit
-- K2 random place   K3 pause
-- pads: root note
-- (params: scale, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local LIMIT = 20000
local UNITS = { 1 / 16, 1 / 8, 1 / 6 }
local UNIT_NAMES = { "1/64", "1/32", "1/24" }
local P = {}
local idx = 1
local root = 50
local scale = {}
local paused = false
local gap = 0

local comp = {}

local function sieve()
  for i = 2, LIMIT do
    if not comp[i] then
      P[#P + 1] = i
      for j = i * i, LIMIT, i do comp[j] = true end
    end
  end
end

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(root, params:get("scale"), 15)
end

local function play()
  local p, nxt = P[idx], P[idx + 1]
  gap = nxt - p
  local note = scale[p % #scale + 1]
  local twin = gap == 2
  engine.amp(twin and 0.3 or 0.22)
  engine.release(params:get("release") * (0.6 + math.min(gap, 20) / 12))
  engine.pan(((p % 7) - 3) / 5)
  engine.hz(MusicUtil.note_num_to_freq(note))
  -- a low tone marks primes that open a long gap
  if gap >= 10 then
    engine.amp(0.18)
    engine.hz(MusicUtil.note_num_to_freq(root - 12))
  end
end

function init()
  sieve()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("PRIMES")
  params:add_option("scale", "scale", names, 12)
  params:set_action("scale", build_scale)
  params:add_option("unit", "gap unit", UNIT_NAMES, 2)
  params:add_control("release", "release", controlspec.new(0.1, 2, 'exp', 0, 0.5, 's'))
  params:add_control("cutoff", "cutoff", controlspec.new(300, 8000, 'exp', 0, 1800, 'hz'))
  params:set_action("cutoff", function(x) engine.cutoff(x) end)
  params:default()
  engine.pw(0.4)
  build_scale()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then root = msg.note - 10 build_scale() end
  end
  clock.run(function()
    while true do
      local wait = 1 / 4
      if not paused then
        play()
        -- long gaps are capped so the walk never stalls
        wait = math.min(gap, 16) * UNITS[params:get("unit")]
        idx = idx + 1
        if idx >= #P then idx = 1 end
      end
      redraw()
      clock.sleep(wait * clock.get_beat_sec())
    end
  end)
end

function enc(n, d)
  if n == 2 then idx = util.clamp(idx + d * 25, 1, #P - 1)
  elseif n == 3 then params:delta("unit", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then idx = math.random(1, #P - 200)
  elseif n == 3 then paused = not paused end
  redraw()
end

function redraw()
  screen.clear()
  screen.level(15)
  screen.move(0, 8)
  screen.text("primes")
  screen.move(127, 8)
  screen.text_right(tostring(P[idx]))
  -- a window of the number line: primes stand tall, gaps show as space
  local base = P[idx] - 20
  for x = 0, 127 do
    local n = base + x // 2
    if x % 2 == 0 and n >= 2 then
      if n <= LIMIT and not comp[n] then
        screen.level(n == P[idx] and 15 or (n < P[idx] and 4 or 9))
        screen.rect(x, n == P[idx] and 16 or 26, 1, n == P[idx] and 30 or 20)
        screen.fill()
      end
    end
  end
  screen.level(2)
  screen.move(0, 47)
  screen.line(127, 47)
  screen.stroke()
  screen.level(4)
  screen.move(0, 62)
  screen.text("gap " .. gap .. "  #" .. idx)
  screen.move(127, 62)
  screen.text_right(paused and "paused" or params:string("unit"))
  screen.update()
end
