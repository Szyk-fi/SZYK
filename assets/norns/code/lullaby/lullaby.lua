-- lullaby
-- a Portamax norns script
--
-- a little tune in three,
-- sung softer and slower each
-- time round, until the cradle
-- is still. then a new one
-- begins.
--
-- E2 tempo      E3 verses
-- K2 new tune   K3 begin again
-- pads: a star of your own
-- (params: key, hush, rock)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local melody = {}  -- 16 bars x 3 beats: { note or false, len }
local chords = {}  -- per bar: scale degree (0-based)
local bar, beat = 1, 1
local verse = 1
local progress = 0 -- 0 .. 1 through the whole slowing-down
local stars = {}
local rock = 0
local resting = false

local scale = {}

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("key") - 12, "Major", 22)
end

-- a 4-bar phrase over the given chords, rhythm in quarter notes
local RHYTHMS = { { 2, 1 }, { 1, 1, 1 }, { 3 }, { 1, 2 }, { 2, 1 } }

local function phrase(chs, start_deg, end_deg)
  local p = {}
  local deg = start_deg
  for b = 1, 4 do
    local r = (b == 4) and { 3 } or RHYTHMS[math.random(#RHYTHMS)]
    local bt = {}
    for i, len in ipairs(r) do
      if b == 4 then
        deg = end_deg
      elseif i == 1 then
        -- strong beat: nearest chord tone
        local ch = chs[b]
        local best, bd = deg, 99
        for _, c in ipairs({ ch, ch + 2, ch + 4, ch + 7, ch + 9 }) do
          if c >= 7 and c <= 16 and math.abs(c - deg) < bd then best, bd = c, math.abs(c - deg) end
        end
        deg = best
      else
        deg = util.clamp(deg + ({ -1, 1, -1, 1, 2, -2 })[math.random(6)], 7, 16)
      end
      bt[#bt + 1] = { deg = deg, len = len }
    end
    p[b] = bt
  end
  return p
end

local function new_tune()
  -- degrees count from the tonic an octave up (7): 8 is the second,
  -- 11 the fifth
  local A1 = { 0, 3, 0, 4 }
  local A2 = { 0, 3, 4, 0 }
  local B = { 3, 0, 1, 4 }
  local a1 = phrase(A1, 9, 8)         -- ends on the second over V: open
  local a2 = phrase(A2, 9, 7)         -- ends home
  local b1 = phrase(B, 10, 11)        -- ends on V's root, waiting
  chords = {}
  melody = {}
  local form = { { a1, A1 }, { a2, A2 }, { b1, B }, { a2, A2 } }
  for _, f in ipairs(form) do
    for b = 1, 4 do
      melody[#melody + 1] = f[1][b]
      chords[#chords + 1] = f[2][b]
    end
  end
  bar, beat = 1, 1
  progress = 0
  verse = 1
  resting = false
end

local function hush()
  return 1 - progress * params:get("hush")
end

local function play(n, amp, rel, cut, pan)
  engine.pan(pan)
  engine.pw(0.5)
  engine.cutoff(cut)
  engine.release(rel)
  engine.amp(amp)
  engine.hz(MusicUtil.note_num_to_freq(n))
end

local function beat_len()
  -- ritardando: the tempo sags to half by the end
  return 60 / (params:get("bpm") * (1 - 0.5 * progress))
end

function init()
  params:add_separator("LULLABY")
  params:add_number("key", "key", 55, 72, 65, function(p) return MusicUtil.note_num_to_name(p:get(), false) end)
  params:set_action("key", build_scale)
  params:add_number("bpm", "tempo", 40, 120, 84)
  params:add_number("verses", "verses", 1, 6, 3)
  params:add_control("hush", "hush", controlspec.new(0, 0.95, 'lin', 0, 0.85, ''))
  params:add_control("rock", "rock", controlspec.new(0, 1, 'lin', 0, 0.6, ''))
  params:default()
  engine.gain(0.4)
  math.randomseed(os.time())
  build_scale()
  new_tune()
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      play(msg.note + 12, 0.15, 2.5, 2400, 0.3)
      table.insert(stars, { x = math.random(70, 124), y = math.random(4, 30), life = 40 })
    end
  end
  clock.run(function()
    while true do
      if resting then
        clock.sleep(2.5)
        new_tune()
      end
      local h = hush()
      local bt = melody[bar]
      local ch = chords[bar]
      -- find the melody note that starts on this beat
      local at = 1
      for _, nt in ipairs(bt) do
        if at == beat then
          local n = scale[nt.deg + 1]
          play(n, 0.24 * h, 0.6 + nt.len * 0.5 * (1 + progress), 1800 * (1 - 0.5 * progress) + 300, 0.1)
          table.insert(stars, { x = 70 + nt.deg * 3, y = 34 - (nt.deg - 7) * 3, life = 24 })
        end
        at = at + nt.len
      end
      -- accompaniment: low root on one, a soft pair of chord tones on two and three
      if beat == 1 then
        play(scale[ch + 1], 0.2 * h, 1.8, 500, -0.2)
      else
        play(scale[ch + 1 + 2 + ((beat == 3) and 2 or 0)] + 12, 0.07 * h, 0.7, 900, -0.35)
      end
      rock = rock + 1
      clock.sleep(beat_len())
      beat = beat + 1
      if beat > 3 then
        beat = 1
        bar = bar + 1
        if bar > #melody then
          bar = 1
          verse = verse + 1
        end
        local total = #melody * params:get("verses")
        progress = math.min(1, ((verse - 1) * #melody + bar - 1) / total)
        if verse > params:get("verses") then resting = true end
      end
    end
  end)
  local frame = metro.init(function()
    for i = #stars, 1, -1 do
      stars[i].life = stars[i].life - 1
      if stars[i].life <= 0 then table.remove(stars, i) end
    end
    redraw()
  end, 1 / 20)
  frame:start()
end

function enc(n, d)
  if n == 2 then params:delta("bpm", d)
  elseif n == 3 then params:delta("verses", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_tune()
  elseif n == 3 then
    bar, beat, verse, progress, resting = 1, 1, 1, 0, false
  end
  redraw()
end

function redraw()
  screen.clear()
  local glow = math.floor(12 * hush()) + 3
  -- moon
  screen.level(glow)
  screen.circle(108, 14, 8)
  screen.fill()
  screen.level(0)
  screen.circle(112, 12, 7)
  screen.fill()
  -- stars, one per note sung
  for _, s in ipairs(stars) do
    screen.level(math.min(15, math.floor(s.life / 2) + 1))
    screen.move(s.x - 2, s.y)
    screen.line(s.x + 2, s.y)
    screen.stroke()
    screen.move(s.x, s.y - 2)
    screen.line(s.x, s.y + 2)
    screen.stroke()
  end
  -- the cradle rocks with the beat, less as it settles
  local t = util.time()
  local amt = params:get("rock") * (resting and 0 or (1 - progress * 0.8))
  local a = math.sin(t * math.pi * 2 / (beat_len() * 6)) * 0.35 * amt
  local cx, cy = 34, 40
  screen.level(glow)
  screen.line_width(1)
  local pts = {}
  for k = 0, 10 do
    local th = math.pi * k / 10
    local x, y = -math.cos(th) * 18, math.sin(th) * 9
    pts[#pts + 1] = { cx + x * math.cos(a) - y * math.sin(a), cy + x * math.sin(a) + y * math.cos(a) }
  end
  for k, p in ipairs(pts) do
    if k == 1 then screen.move(p[1], p[2]) else screen.line(p[1], p[2]) end
  end
  screen.close()
  screen.stroke()
  -- rockers
  screen.level(4)
  screen.move(10, 58)
  screen.line(58, 58)
  screen.stroke()
  -- bar dots: where we are in the tune
  for b = 1, #melody do
    screen.level(b == bar and 15 or (b < bar and 5 or 2))
    screen.pixel(2 + (b - 1) * 4, 62)
    screen.fill()
  end
  screen.level(5)
  screen.move(127, 62)
  screen.text_right(resting and "shh" or ("verse " .. math.min(verse, params:get("verses"))))
  screen.level(math.max(3, glow - 3))
  screen.move(2, 8)
  screen.text("lullaby")
  screen.update()
end
