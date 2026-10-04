-- beatty
-- a Portamax norns script
--
-- floor(n*r) for an irrational r
-- marks some steps; floor(n*s), with
-- 1/r + 1/s = 1, marks exactly the
-- rest. two voices share the beat
-- with no gap and no overlap, and
-- the pattern never quite repeats.
--
-- E2 ratio   E3 speed
-- K2 restart   K3 pause
-- pads: root note
-- (params: scale, release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local RATIOS = {
  { "sqrt 2", math.sqrt(2) }, { "phi", (1 + math.sqrt(5)) / 2 },
  { "sqrt 3", math.sqrt(3) }, { "e", math.exp(1) }, { "pi", math.pi },
}
local DIVS = { 1 / 4, 1 / 3, 1 / 2 }
local step = 0
local owner = {} -- owner[k] = 1 or 2 for step k
local count = { 0, 0 }
local scale = {}
local root = 45
local paused = false

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(root, params:get("scale"), 22)
end

local function build()
  local r = RATIOS[params:get("ratio")][2]
  local s = r / (r - 1)
  owner = {}
  for n = 1, 1000 do
    local a, b = math.floor(n * r), math.floor(n * s)
    if a <= 1000 then owner[a] = 1 end
    if b <= 1000 then owner[b] = 2 end
  end
end

local function tick()
  step = step % 1000 + 1
  local who = owner[step] or 1
  count[who] = count[who] + 1
  local note
  if who == 1 then
    -- low voice climbs in thirds and folds back
    note = scale[(count[1] * 2) % 7 + 1]
    engine.pan(-0.4)
    engine.pw(0.5)
    engine.amp(0.28)
  else
    -- high voice steps downward through the upper octave
    note = scale[15 - (count[2] % 8)]
    engine.pan(0.4)
    engine.pw(0.25)
    engine.amp(0.18)
  end
  engine.hz(MusicUtil.note_num_to_freq(note))
end

function init()
  local rnames = {}
  for i, r in ipairs(RATIOS) do rnames[i] = r[1] end
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("BEATTY")
  params:add_option("ratio", "ratio r", rnames, 1)
  params:set_action("ratio", build)
  params:add_option("speed", "speed", { "1/16", "1/8t", "1/8" }, 2)
  params:add_option("scale", "scale", names, 5)
  params:set_action("scale", build_scale)
  params:add_control("release", "release", controlspec.new(0.05, 2, 'exp', 0, 0.35, 's'))
  params:set_action("release", function(x) engine.release(x) end)
  params:default()
  engine.cutoff(1800)
  build_scale()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then root = msg.note - 15 build_scale() end
  end
  clock.run(function()
    while true do
      if not paused then tick() end
      redraw()
      clock.sync(DIVS[params:get("speed")])
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("ratio", d)
  elseif n == 3 then params:delta("speed", -d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then step = 0 count = { 0, 0 }
  elseif n == 3 then paused = not paused end
  redraw()
end

function redraw()
  screen.clear()
  screen.level(15)
  screen.move(0, 8)
  screen.text("beatty")
  screen.level(4)
  screen.move(127, 8)
  screen.text_right("r = " .. RATIOS[params:get("ratio")][1])
  -- two lanes, 32 steps either side of now
  for i = -15, 16 do
    local k = step + i
    if k >= 1 and k <= 1000 then
      local who = owner[k] or 1
      local x = 62 + i * 4
      screen.level(i == 0 and 15 or (i < 0 and 4 or 8))
      screen.rect(x, who == 1 and 40 or 22, 3, 6)
      screen.fill()
    end
  end
  screen.level(2)
  screen.move(63, 16)
  screen.line(63, 50)
  screen.stroke()
  screen.level(4)
  screen.move(0, 62)
  screen.text("low " .. count[1] .. "  high " .. count[2])
  screen.move(127, 62)
  screen.text_right(paused and "paused" or ("n" .. step))
  screen.update()
end
