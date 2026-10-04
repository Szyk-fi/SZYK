-- gamelan
-- a Portamax norns script
--
-- a sixteen-beat gong cycle. a
-- slow core melody walks around
-- it while two parts, polos and
-- sangsih, interlock four notes
-- to the beat, each filling the
-- other's gaps. paired voices are
-- tuned a few hz apart so the
-- metal shimmers (ombak).
--
-- E2 tempo   E3 brightness
-- K2 new melody   K3 kotekan on/off
-- pads: set the next core note
-- (params: tuning, base, ombak,
--  tempo)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

-- approximate cents above the tonic. no two real gamelan share a
-- tuning, so these are typical shapes rather than one set's values
local TUNINGS = {
  { name = "slendro", cents = { 0, 240, 480, 720, 960 } },
  { name = "pelog", cents = { 0, 120, 270, 540, 670, 785, 950 } },
}

local CYCLE = 16
local core = {}
local beat = 0
local sub = 0
local kotekan_on = true
local polos_hist = {}
local sangsih_hist = {}
local gong_glow = 0
local beat_glow = 0
local pending = nil

local function cents()
  return TUNINGS[params:get("tuning")].cents
end

-- degree d counts scale tones from the tonic, across octaves
local function hz(d)
  local c = cents()
  local n = #c
  local oct = math.floor(d / n)
  local i = d - oct * n
  local base = MusicUtil.note_num_to_freq(params:get("base"))
  return base * 2 ^ (oct + c[i + 1] / 1200)
end

local function strike(d, pan, pw, amp, rel, cutoff, pair)
  engine.pan(pan)
  engine.pw(pw)
  engine.amp(amp)
  engine.release(rel)
  engine.cutoff(cutoff)
  local f = hz(d)
  engine.hz(f)
  -- the paired instrument, tuned slightly sharp
  if pair then engine.hz(f + params:get("ombak")) end
end

local function new_core()
  local n = #cents()
  core = {}
  local d = 0
  for i = 1, CYCLE do
    d = util.clamp(d + ({ -2, -1, 1, 2, 1, -1 })[math.random(1, 6)], -1, n + 1)
    core[i] = d
  end
  -- every phrase lands somewhere stable, and the cycle lands home
  core[8] = math.floor(n / 2)
  core[CYCLE] = 0
end

local function on_sub()
  local b = beat % CYCLE + 1
  local bright = params:get("bright")
  if sub == 0 then
    local d = core[b]
    if pending then
      d = pending
      core[b] = d
      pending = nil
    end
    strike(d, 0, 0.3, 0.2, 1.4, bright * 0.6, true)
    beat_glow = 15
    -- kenong marks each quarter of the cycle
    if b % 4 == 0 then strike(d - #cents(), 0.3, 0.45, 0.18, 2.5, bright * 0.4, false) end
    if b == CYCLE then
      strike(-2 * #cents(), 0, 0.5, 0.4, 7, 350, true)
      gong_glow = 15
    end
  end
  if kotekan_on then
    -- three-tone interlock around the next core note: polos owns
    -- the lower two, sangsih the upper two, they share the middle
    local target = core[(b % CYCLE) + 1] + #cents()
    local pattern = (beat % 2 == 0) and { 0, 1, -1, 0 } or { 1, -1, 0, 1 }
    local off = pattern[sub + 1]
    local p, s = nil, nil
    if off <= 0 then p = target + off end
    if off >= 0 then s = target + off end
    if p then strike(p, -0.55, 0.5, 0.12, 0.35, bright, false) end
    if s then strike(s, 0.55, 0.5, 0.12, 0.35, bright, false) end
    table.insert(polos_hist, p ~= nil)
    table.insert(sangsih_hist, s ~= nil)
  else
    table.insert(polos_hist, false)
    table.insert(sangsih_hist, false)
  end
  if #polos_hist > 16 then table.remove(polos_hist, 1) end
  if #sangsih_hist > 16 then table.remove(sangsih_hist, 1) end
  sub = sub + 1
  if sub == 4 then
    sub = 0
    beat = beat + 1
  end
end

function init()
  local tn = {}
  for i, t in ipairs(TUNINGS) do tn[i] = t.name end
  params:add_separator("GAMELAN")
  params:add_option("tuning", "tuning", tn, 1)
  params:set_action("tuning", function() new_core() end)
  params:add_number("base", "base", 40, 64, 50, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:add_number("tempo", "tempo", 30, 140, 72)
  params:add_control("ombak", "ombak", controlspec.new(0, 10, 'lin', 0, 4, 'hz'))
  params:add_control("bright", "brightness", controlspec.new(300, 8000, 'exp', 0, 3000, 'hz'))
  params:default()
  engine.gain(1.0)
  math.randomseed(os.time())
  new_core()
  -- start one beat before the gong, so the cycle opens with it
  beat = CYCLE - 1
  on_sub()

  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      local n = #cents()
      pending = (msg.note - params:get("base")) % 12 * n // 12
      strike(pending + n, 0, 0.4, 0.18, 0.8, params:get("bright"), true)
    end
  end

  clock.run(function()
    while true do
      clock.sleep(60 / params:get("tempo") / 4)
      on_sub()
    end
  end)
  local mt = metro.init(function()
    gong_glow = math.max(0, gong_glow - 0.25)
    beat_glow = math.max(0, beat_glow - 1)
    redraw()
  end, 1 / 20)
  mt:start()
end

function enc(n, d)
  if n == 2 then params:delta("tempo", d)
  elseif n == 3 then params:delta("bright", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_core()
  elseif n == 3 then kotekan_on = not kotekan_on end
end

function redraw()
  screen.clear()
  local cx, cy, r = 31, 31, 22
  local n = #cents()
  local cur = (beat - (sub == 0 and 1 or 0)) % CYCLE + 1
  -- the gong in the middle, swelling when struck
  screen.level(math.floor(2 + gong_glow * 0.8))
  screen.circle(cx, cy, 4 + gong_glow / 4)
  screen.fill()
  -- core melody around the cycle: farther out is higher
  for i = 1, CYCLE do
    local a = -math.pi / 2 + (i - 1) * 2 * math.pi / CYCLE
    local rr = r - 6 + util.clamp(core[i], -1, n + 1) * 8 / (n + 1)
    local x, y = cx + math.cos(a) * rr, cy + math.sin(a) * rr
    if i == cur then
      screen.level(math.max(10, beat_glow))
      screen.circle(x, y, 2.5)
      screen.fill()
    else
      screen.level(i % 4 == 0 and 7 or 3)
      screen.circle(x, y, 1.2)
      screen.fill()
    end
  end
  -- interlocking parts, newest on the right
  local rows = { { "polos", polos_hist, 24 }, { "sangsih", sangsih_hist, 40 } }
  for _, row in ipairs(rows) do
    screen.level(4)
    screen.move(64, row[3] - 5)
    screen.text(row[1])
    for i, on in ipairs(row[2]) do
      local x = 64 + (i - 1) * 4
      if on then
        screen.level(i == #row[2] and 15 or 8)
        screen.rect(x, row[3], 3, 4)
        screen.fill()
      else
        screen.level(1)
        screen.rect(x, row[3] + 3, 3, 1)
        screen.fill()
      end
    end
  end
  screen.level(15)
  screen.move(64, 8)
  screen.text("gamelan")
  screen.level(5)
  screen.move(64, 62)
  screen.text(TUNINGS[params:get("tuning")].name)
  screen.move(127, 62)
  screen.text_right(params:get("tempo") .. (kotekan_on and "" or " -"))
  screen.update()
end
