-- walkingbass
-- a Portamax norns script
--
-- a bass walks four to the bar
-- through ii-V-I changes that
-- drift from key to key, with a
-- ride cymbal swinging on top.
--
-- E2 tempo      E3 swing
-- K2 new tune   K3 mute ride
-- pads: blow over the changes
-- (params: key, ride level, comp)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local KEYS = { "C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B" }
local QUAL = {
  m7 = { 0, 3, 7, 10 }, ["7"] = { 0, 4, 7, 10 }, maj7 = { 0, 4, 7, 11 },
  m7b5 = { 0, 3, 6, 10 }, ["7b9"] = { 0, 4, 7, 10 },
}
-- scale used for passing tones over each quality
local PASS = {
  m7 = { 0, 2, 3, 5, 7, 9, 10 }, ["7"] = { 0, 2, 4, 5, 7, 9, 10 }, maj7 = { 0, 2, 4, 5, 7, 9, 11 },
  m7b5 = { 0, 1, 3, 5, 6, 8, 10 }, ["7b9"] = { 0, 1, 4, 5, 7, 8, 10 },
}

local tune = {}     -- list of { root = pc, q = quality }
local bar = 1
local beat = 0
local walk = {}     -- recent bass notes for drawing
local last_note = 40
local ride_on = true
local ride_flash = 0
local solo = {}

local function add_251(key, minor)
  if minor then
    table.insert(tune, { root = (key + 2) % 12, q = "m7b5" })
    table.insert(tune, { root = (key + 7) % 12, q = "7b9" })
    table.insert(tune, { root = key, q = "m7" })
    table.insert(tune, { root = key, q = "m7" })
  else
    table.insert(tune, { root = (key + 2) % 12, q = "m7" })
    table.insert(tune, { root = (key + 7) % 12, q = "7" })
    table.insert(tune, { root = key, q = "maj7" })
    table.insert(tune, { root = (key + 9) % 12, q = "7" }) -- VI7 turns back around
  end
end

local function new_tune()
  tune = {}
  local key = (params:get("key") - 1)
  -- four ii-V-Is; each moves the key down a whole step or up a fourth
  for i = 1, 4 do
    add_251(key, math.random() < 0.3)
    key = (key + (math.random() < 0.5 and 10 or 5)) % 12
  end
  bar = 1
end

local function chord_name(c)
  return KEYS[c.root + 1] .. c.q
end

-- nearest note with pitch class pc to `from`, kept in the bass range
local function nearest(pc, from)
  local best, bd = nil, 99
  for n = 28, 55 do
    if n % 12 == pc then
      local d = math.abs(n - from)
      if d < bd then best, bd = n, d end
    end
  end
  return best
end

local function in_range(n)
  while n < 28 do n = n + 12 end
  while n > 55 do n = n - 12 end
  return n
end

-- the four notes of one bar, heading towards the next bar's root
local function walk_bar(c, nxt)
  local notes = {}
  local root = nearest(c.root, last_note)
  local target = nearest(nxt.root, root + (math.random() < 0.5 and 5 or -5))
  notes[1] = root
  local pass = PASS[c.q]
  local tones = QUAL[c.q]
  -- beat 2 and 3: chord tones or scale steps moving towards the target
  local dir = target >= root and 1 or -1
  local function step_from(n, prefer_chord)
    local cands = {}
    for _, iv in ipairs(prefer_chord and tones or pass) do
      for o = -12, 12, 12 do
        local m = nearest(c.root, n) + iv + o
        if (m - n) * dir > 0 and math.abs(m - n) <= (prefer_chord and 7 or 4) then cands[#cands + 1] = m end
      end
    end
    if #cands == 0 then return in_range(n + dir * 2) end
    table.sort(cands, function(a, b) return math.abs(a - n) < math.abs(b - n) end)
    return in_range(cands[math.random(1, math.min(2, #cands))])
  end
  notes[2] = step_from(root, math.random() < 0.6)
  notes[3] = step_from(notes[2], math.random() < 0.4)
  -- beat 4: a chromatic approach into the next root
  local appr = target + ((notes[3] < target) and -1 or 1)
  if math.random() < 0.25 then appr = target + 7 end -- or its fifth
  notes[4] = in_range(appr)
  return notes, target
end

local function bass(n, accent)
  engine.pan(-0.15)
  engine.pw(0.45)
  engine.cutoff(params:get("bass_tone"))
  engine.release(0.55)
  engine.amp(accent and 0.62 or 0.5)
  engine.hz(MusicUtil.note_num_to_freq(n))
  table.insert(walk, n)
  while #walk > 24 do table.remove(walk, 1) end
end

local function ride(loud)
  if not ride_on then return end
  engine.pan(0.45)
  engine.pw(0.12)
  engine.cutoff(12000)
  engine.release(loud and 0.18 or 0.08)
  local a = params:get("ride") * (loud and 1 or 0.6)
  engine.amp(a)
  engine.hz(5274 * (0.99 + math.random() * 0.02))
  engine.amp(a * 0.6)
  engine.hz(7459 * (0.99 + math.random() * 0.02))
  ride_flash = loud and 15 or 8
end

local function comp(c)
  if params:get("comp") == 0 then return end
  -- shell voicing: the third and seventh, up in the middle register
  local t = QUAL[c.q]
  engine.pan(0.1)
  engine.pw(0.5)
  engine.cutoff(1400)
  engine.release(0.35)
  engine.amp(0.09)
  for i = 2, 4, 2 do
    local n = 52 + ((c.root + t[i]) - 52) % 12
    engine.hz(MusicUtil.note_num_to_freq(n))
  end
end

function init()
  params:add_separator("WALKINGBASS")
  params:add_option("key", "first key", KEYS, 1)
  params:add_number("bpm", "tempo", 60, 280, 152)
  params:add_control("swing", "swing", controlspec.new(0.5, 0.75, 'lin', 0, 0.64, ''))
  params:add_control("ride", "ride level", controlspec.new(0, 0.3, 'lin', 0, 0.09, ''))
  params:add_control("bass_tone", "bass tone", controlspec.new(200, 2000, 'exp', 0, 700, 'hz'))
  params:add_binary("comp", "comp", "toggle", 1)
  params:default()
  engine.gain(1.2)
  math.randomseed(os.time())
  new_tune()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      -- pads snap to the current chord's scale
      local c = tune[bar]
      local n = MusicUtil.snap_note_to_array(msg.note + 12, (function()
        local a = {}
        for o = 0, 8 do for _, iv in ipairs(PASS[c.q]) do a[#a + 1] = o * 12 + c.root + iv end end
        return a
      end)())
      engine.pan(0)
      engine.pw(0.3)
      engine.cutoff(2500)
      engine.release(0.6)
      engine.amp(0.25)
      engine.hz(MusicUtil.note_num_to_freq(n))
      table.insert(solo, { n = n, life = 20 })
    end
  end
  clock.run(function()
    while true do
      local c = tune[bar]
      local nxt = tune[bar % #tune + 1]
      local notes, target = walk_bar(c, nxt)
      for b = 1, 4 do
        beat = b
        local spb = 60 / params:get("bpm")
        local sw = params:get("swing")
        bass(notes[b], b == 1)
        -- ride: ding, ding-a, ding, ding-a
        ride(b % 2 == 1)
        if b == 2 and math.random() < 0.7 then comp(c) end
        if b % 2 == 0 then
          clock.sleep(spb * sw)
          ride(false)
          if math.random() < 0.3 then comp(c) end
          clock.sleep(spb * (1 - sw))
        else
          clock.sleep(spb)
        end
      end
      last_note = notes[4]
      bar = bar % #tune + 1
    end
  end)
  local frame = metro.init(function()
    ride_flash = math.max(0, ride_flash - 2)
    for i = #solo, 1, -1 do
      solo[i].life = solo[i].life - 1
      if solo[i].life <= 0 then table.remove(solo, i) end
    end
    redraw()
  end, 1 / 24)
  frame:start()
end

function enc(n, d)
  if n == 2 then params:delta("bpm", d)
  elseif n == 3 then params:delta("swing", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_tune()
  elseif n == 3 then ride_on = not ride_on end
  redraw()
end

function redraw()
  screen.clear()
  -- the changes: four bars around "now"
  local first = math.floor((bar - 1) / 4) * 4 + 1
  for i = 0, 3 do
    local b = first + i
    local c = tune[b]
    if c then
      local x = 2 + i * 32
      screen.level(b == bar and 15 or 4)
      screen.move(x, 9)
      screen.text(chord_name(c))
      if b == bar then
        for k = 1, 4 do
          screen.level(k <= beat and 12 or 2)
          screen.rect(x + (k - 1) * 6, 12, 4, 2)
          screen.fill()
        end
      end
    end
  end
  -- the walking line as a staircase
  screen.level(2)
  for y = 24, 56, 8 do
    screen.move(0, y)
    screen.line(104, y)
    screen.stroke()
  end
  for i, n in ipairs(walk) do
    local x = (i - 1) * 4 + 4
    local y = util.linlin(28, 55, 58, 22, n)
    screen.level(i == #walk and 15 or math.max(2, math.floor(i / #walk * 10)))
    screen.rect(x, y - 1, 3, 2)
    screen.fill()
  end
  for _, s in ipairs(solo) do
    screen.level(math.floor(s.life / 2))
    screen.circle(96, util.linlin(48, 96, 58, 22, s.n), 2)
    screen.stroke()
  end
  -- the ride cymbal
  screen.level(ride_on and math.max(3, ride_flash) or 1)
  screen.move(108, 30)
  screen.line(127, 27)
  screen.stroke()
  screen.move(117, 30)
  screen.line(117, 50)
  screen.stroke()
  screen.level(5)
  screen.move(127, 62)
  screen.text_right(params:get("bpm"))
  screen.update()
end
