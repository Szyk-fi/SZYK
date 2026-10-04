-- musicbox
-- a Portamax norns script
--
-- a pinned cylinder turns past
-- a steel comb. the spring runs
-- down and the tune slows with
-- it, until you wind it again.
--
-- E2 spring tension  E3 key
-- K2 punch new pins  K3 wind up
-- pads: pluck a tine
-- (params: tune style, auto-wind)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local TINES = 15
local COLS = 32
local pins = {}       -- pins[col] = { tine indices }
local notes = {}
local pos = 0         -- cylinder position in columns (fractional)
local speed = 0       -- columns per second
local ring = {}       -- tine vibration for drawing
for t = 1, TINES do ring[t] = 0 end
local stopped_for = 0
local wound = 1

local function build_notes()
  local style = params:get("style")
  local scale = style == 2 and "Natural Minor" or (style == 3 and "Major Pentatonic" or "Major")
  notes = MusicUtil.generate_scale_of_length(params:get("key"), scale, TINES)
end

-- tine index (1-based) of scale degree d (0-based) in the comb
local function chord_tines(root_deg)
  return { root_deg + 1, root_deg + 3, root_deg + 5 }
end

local function punch()
  pins = {}
  for c = 1, COLS do pins[c] = {} end
  -- four bars of eight steps; a simple progression underneath
  local prog = ({ { 0, 3, 4, 0 }, { 0, 5, 3, 4 }, { 0, 4, 5, 3 } })[math.random(3)]
  local mel = 7 + math.random(0, 3)
  for b = 0, 3 do
    local ch = chord_tines(prog[b + 1])
    for s = 0, 7 do
      local c = b * 8 + s + 1
      if s == 0 then table.insert(pins[c], ch[1]) end
      if s == 4 and math.random() < 0.7 then table.insert(pins[c], ch[2 + math.random(0, 1)]) end
      -- melody: a random walk over the upper tines, landing on chord
      -- tones on strong steps
      if s % 2 == 0 or math.random() < 0.35 then
        mel = util.clamp(mel + math.random(-2, 2), 6, TINES)
        if s % 4 == 0 then
          local best, bd = mel, 99
          for _, t in ipairs(ch) do
            for o = 0, 14, 7 do
              local tt = t + o
              if tt >= 6 and tt <= TINES and math.abs(tt - mel) < bd then best, bd = tt, math.abs(tt - mel) end
            end
          end
          mel = best
        end
        table.insert(pins[c], mel)
      end
    end
  end
end

local function pluck(t, vel)
  local n = notes[t]
  engine.pan(util.linlin(1, TINES, -0.6, 0.6, t))
  engine.pw(0.5)
  engine.cutoff(3000 + t * 250)
  engine.release(util.linlin(1, TINES, 2.6, 1.0, t))
  engine.amp(0.22 * vel)
  engine.hz(MusicUtil.note_num_to_freq(n + 12))
  -- the bright metallic overtone of a struck tine
  engine.amp(0.04 * vel)
  engine.release(0.3)
  engine.hz(MusicUtil.note_num_to_freq(n + 12) * 4.07)
  ring[t] = 15
end

local function wind()
  wound = 1
  stopped_for = 0
end

local function update(dt)
  -- the governor keeps speed steady while the spring is strong; as it
  -- runs down the tune drags
  wound = math.max(0, wound - dt / params:get("tension"))
  local full = params:get("speed")
  speed = full * math.min(1, wound * 3) ^ 0.8
  local before = pos
  pos = pos + speed * dt
  local vel = 0.5 + 0.5 * math.min(1, wound * 3)
  for c = math.floor(before) + 1, math.floor(pos) do
    local col = (c % COLS) + 1
    for _, t in ipairs(pins[col]) do pluck(t, vel) end
  end
  if speed < 0.05 then
    stopped_for = stopped_for + dt
    if params:get("autowind") == 1 and stopped_for > 3 then wind() end
  end
  for t = 1, TINES do ring[t] = math.max(0, (ring[t] or 0) - 1) end
end

function init()
  params:add_separator("MUSICBOX")
  params:add_number("key", "key", 60, 72, 65, function(p) return MusicUtil.note_num_to_name(p:get(), false) end)
  params:set_action("key", build_notes)
  params:add_option("style", "tune style", { "major", "minor", "pentatonic" }, 1)
  params:set_action("style", build_notes)
  params:add_control("speed", "speed", controlspec.new(2, 12, 'lin', 0, 5.5, 'col/s'))
  params:add_control("tension", "spring", controlspec.new(10, 120, 'lin', 1, 45, 's'))
  params:add_option("autowind", "auto-wind", { "on", "off" }, 1)
  params:default()
  engine.gain(0.8)
  math.randomseed(os.time())
  build_notes()
  punch()
  wind()
  -- start just before a column so the first pins ring at once
  pos = -0.05
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then pluck(util.clamp(msg.note - 59, 1, TINES), 1) end
  end
  local last = util.time()
  local frame = metro.init(function()
    local now = util.time()
    update(math.min(0.1, now - last))
    last = now
    redraw()
  end, 1 / 40)
  frame:start()
end

function enc(n, d)
  if n == 2 then params:delta("tension", d)
  elseif n == 3 then params:delta("key", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then punch()
  elseif n == 3 then wind() end
  redraw()
end

function redraw()
  screen.clear()
  -- the cylinder: a band of columns scrolling left into the comb at x=96
  local CX = 96
  local frac = pos - math.floor(pos)
  screen.level(1)
  screen.rect(8, 4, CX - 8, 46)
  screen.stroke()
  for k = -1, 13 do
    local c = math.floor(pos) + k
    local col = ((c % COLS) + COLS) % COLS + 1
    local x = CX - (k + 1 - frac) * 6.5
    if x > 8 and x < CX + 1 then
      -- columns curve away from you near the edges of the cylinder
      local depth = math.abs((x - 52) / 46)
      for _, t in ipairs(pins[col]) do
        screen.level(math.max(1, math.floor(12 - depth * 9)))
        screen.rect(x, 50 - t * 3, 2, 2)
        screen.fill()
      end
      if col == 1 then
        screen.level(2)
        screen.move(x, 5)
        screen.line(x, 49)
        screen.stroke()
      end
    end
  end
  -- the comb: tines of decreasing length, trembling when plucked
  for t = 1, TINES do
    local y = 50 - t * 3 + 1
    local len = 30 - t
    local wob = (ring[t] > 0) and ((ring[t] % 2) * 2 - 1) or 0
    screen.level(ring[t] > 0 and math.max(4, ring[t]) or 3)
    screen.move(CX + 1, y)
    screen.line(CX + 1 + len, y + wob * 0.6)
    screen.stroke()
  end
  -- spring gauge
  screen.level(3)
  screen.rect(8, 56, 60, 5)
  screen.stroke()
  screen.level(wound > 0.33 and 10 or 15)
  screen.rect(9, 57, math.max(0, 58 * wound), 3)
  screen.fill()
  screen.level(15)
  screen.move(127, 62)
  screen.text_right(speed < 0.05 and "wind me" or "musicbox")
  screen.update()
end
