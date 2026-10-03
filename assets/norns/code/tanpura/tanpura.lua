-- tanpura
-- a Portamax norns script
--
-- four drone strings are plucked
-- in the old order, pa sa sa sa,
-- each left to ring a long time.
-- above them a slow voice wanders
-- a raga-like scale, always
-- finding its way home to sa.
--
-- E2 pace   E3 brightness
-- K2 next raga   K3 voice on/off
-- pads: the voice sings that note
-- (params: raga, sa, cycle,
--  drone release, voice release)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

-- named after the feel, built from the closest musicutil scale
local RAGAS = {
  { name = "yaman", scale = "Lydian" },
  { name = "kafi", scale = "Dorian" },
  { name = "bhairav", scale = "Double Harmonic" },
  { name = "bhupali", scale = "Major Pentatonic" },
  { name = "purvi", scale = "East Indian Purvi" },
  { name = "bhairavi", scale = "Phrygian" },
}

local scale = {}
local strings = {}
local melody_on = true
local degree = 8
local contour = {}
local t = 0

local function build_scale()
  local r = RAGAS[params:get("raga")]
  -- from the sa below the voice's octave, two octaves up
  scale = MusicUtil.generate_scale(params:get("sa"), r.scale, 2)
end

local function steps_per_octave()
  return (#scale - 1) // 2
end

local function pluck(i)
  local s = strings[i]
  engine.pan(s.pan)
  engine.pw(0.08)
  engine.gain(2.2)
  engine.cutoff(params:get("bright") * 0.45)
  engine.release(params:get("drone_rel"))
  engine.amp(0.2)
  engine.hz(MusicUtil.note_num_to_freq(params:get("sa") + s.offset))
  s.amp = 1
end

local function sing(note, amp)
  engine.pan(0.15)
  engine.pw(0.42)
  engine.gain(1.0)
  engine.cutoff(params:get("bright"))
  engine.release(params:get("voice_rel"))
  engine.amp(amp or 0.2)
  engine.hz(MusicUtil.note_num_to_freq(note))
  table.insert(contour, { x = 127, note = note })
end

-- one phrase step: mostly neighbours, sometimes a leap back to
-- sa or pa, which are where a phrase likes to come to rest
local function next_degree()
  local o = steps_per_octave()
  local r = math.random()
  if r < 0.12 then
    return o + 1
  elseif r < 0.2 then
    if math.random() < 0.5 then return 1 end
    local pa = params:get("sa") + 19
    for i, n in ipairs(scale) do if n == pa then return i end end
    return o + 1
  end
  local d = degree + ({ -1, -1, 1, 1, -2, 2 })[math.random(1, 6)]
  return util.clamp(d, 1, #scale)
end

function init()
  local raga_names = {}
  for i, r in ipairs(RAGAS) do raga_names[i] = r.name end
  params:add_separator("TANPURA")
  params:add_option("raga", "raga", raga_names, 1)
  params:set_action("raga", build_scale)
  params:add_number("sa", "sa", 40, 60, 50, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("sa", build_scale)
  params:add_control("pace", "pace", controlspec.new(0.25, 2, 'exp', 0, 0.7, 's'))
  params:add_control("cycle", "cycle", controlspec.new(1.5, 6, 'lin', 0, 3, 's'))
  params:add_control("bright", "brightness", controlspec.new(300, 6000, 'exp', 0, 2200, 'hz'))
  params:add_control("drone_rel", "drone release", controlspec.new(1, 10, 'exp', 0, 5, 's'))
  params:add_control("voice_rel", "voice release", controlspec.new(0.3, 5, 'exp', 0, 1.8, 's'))
  params:default()
  math.randomseed(os.time())
  build_scale()
  -- pa (a fourth below sa), sa, sa, and sa an octave down
  strings = {
    { offset = -5, pan = -0.5, amp = 0, x = 14 },
    { offset = 0, pan = -0.15, amp = 0, x = 24 },
    { offset = 0, pan = 0.15, amp = 0, x = 34 },
    { offset = -12, pan = 0.5, amp = 0, x = 44 },
  }

  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      local n = msg.note
      while n < scale[1] do n = n + 12 end
      while n > scale[#scale] do n = n - 12 end
      n = MusicUtil.snap_note_to_array(n, scale)
      for i, v in ipairs(scale) do if v == n then degree = i end end
      sing(n, 0.24)
    end
  end

  -- the drone: pa sa sa sa, then a breath
  clock.run(function()
    while true do
      local gap = params:get("cycle") / 5
      for i = 1, 4 do
        pluck(i)
        clock.sleep(gap)
      end
      clock.sleep(gap)
    end
  end)
  -- the voice
  clock.run(function()
    clock.sleep(1.2)
    while true do
      local beats = ({ 1, 1, 2, 2, 3, 4 })[math.random(1, 6)]
      if melody_on then
        if math.random() < 0.85 then
          degree = next_degree()
          sing(scale[degree])
        end
      end
      clock.sleep(beats * params:get("pace"))
    end
  end)
  local mt = metro.init(function()
    t = t + 1
    for _, s in ipairs(strings) do s.amp = s.amp * 0.985 end
    for i = #contour, 1, -1 do
      contour[i].x = contour[i].x - 0.6
      if contour[i].x < 56 then table.remove(contour, i) end
    end
    redraw()
  end, 1 / 20)
  mt:start()
end

function enc(n, d)
  if n == 2 then params:delta("pace", d)
  elseif n == 3 then params:delta("bright", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    params:set("raga", params:get("raga") % #RAGAS + 1)
    degree = steps_per_octave() + 1
  elseif n == 3 then
    melody_on = not melody_on
  end
end

function redraw()
  screen.clear()
  -- the strings, shimmering while they ring
  for i, s in ipairs(strings) do
    screen.level(math.floor(2 + s.amp * 13))
    screen.move(s.x, 4)
    for y = 6, 54, 4 do
      local wob = math.sin(y * 0.5 + t * (1.3 + i * 0.4)) * s.amp * 2.5 * math.sin(math.pi * (y - 4) / 50)
      screen.line(s.x + wob, y)
    end
    screen.stroke()
  end
  screen.level(2)
  screen.move(8, 4)
  screen.line(50, 4)
  screen.move(8, 54)
  screen.line(50, 54)
  screen.stroke()
  -- the voice's path, scrolling away to the left
  local lo, hi = scale[1], scale[#scale]
  for _, c in ipairs(contour) do
    local y = util.linlin(lo, hi, 52, 8, c.note)
    screen.level(math.floor(util.linlin(56, 127, 2, 14, c.x)))
    screen.rect(c.x, y, 4, 2)
    screen.fill()
  end
  -- sa guide line
  screen.level(1)
  local sa_y = util.linlin(lo, hi, 52, 8, lo + 12)
  screen.move(58, sa_y + 1)
  screen.line(127, sa_y + 1)
  screen.stroke()
  screen.level(15)
  screen.move(0, 63)
  screen.text("tanpura")
  screen.level(5)
  screen.move(127, 63)
  screen.text_right(RAGAS[params:get("raga")].name .. " " .. MusicUtil.note_num_to_name(params:get("sa")) .. (melody_on and "" or " -"))
  screen.update()
end
