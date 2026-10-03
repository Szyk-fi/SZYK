-- chorale
-- a Portamax norns script
--
-- four voices sing a slow
-- progression. each chord is
-- voiced to move every part as
-- little as it can, with no
-- parallel fifths or octaves.
--
-- E2 tempo       E3 brightness
-- K2 new phrase  K3 cadence now
-- pads: choose the next chord
-- (params: key, mode, arpeggiate)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local NAMES = { "I", "ii", "iii", "IV", "V", "vi", "vii" }
-- where each degree likes to go next (functional harmony, loosely)
local NEXT = {
  { 4, 5, 6, 2, 3 }, { 5, 7, 5 }, { 6, 4 }, { 5, 1, 2, 5 }, { 1, 6, 1, 4 }, { 2, 4, 5 }, { 1, 1, 3 },
}
local RANGE = { { 60, 79 }, { 55, 72 }, { 48, 67 }, { 40, 60 } } -- S A T B

local degree = 1
local voices = { 72, 67, 64, 48 }
local hist = {}
local count = 0
local cadence_at = 8
local forced = nil
local sung = 0

local function scale_pc()
  local mode = params:get("mode") == 1 and "Major" or "Natural Minor"
  local s = MusicUtil.generate_scale_of_length(params:get("key"), mode, 7)
  local pcs = {}
  for i = 1, 7 do pcs[i] = s[i] % 12 end
  if params:get("mode") == 2 then pcs[7] = (pcs[1] + 11) % 12 end -- raised leading tone
  return pcs
end

local function triad(deg)
  local pcs = scale_pc()
  return { pcs[deg], pcs[(deg + 1) % 7 + 1], pcs[(deg + 3) % 7 + 1] }
end

local function cands(pcs, lo, hi)
  local o = {}
  for n = lo, hi do
    for _, pc in ipairs(pcs) do if n % 12 == pc then o[#o + 1] = n end end
  end
  return o
end

local function motion_bad(a1, a2, b1, b2)
  -- parallel perfect fifths or octaves between two voices
  local i1, i2 = (a1 - b1) % 12, (a2 - b2) % 12
  return a1 ~= a2 and (i1 == 7 or i1 == 0) and i1 == i2
end

local function voice(deg)
  local ch = triad(deg)
  local best, bc = nil, math.huge
  local bass_c = {}
  for _, n in ipairs(cands({ ch[1] }, RANGE[4][1], RANGE[4][2])) do bass_c[#bass_c + 1] = n end
  if deg ~= 1 and deg ~= 5 then -- first inversion now and then
    for _, n in ipairs(cands({ ch[2] }, RANGE[4][1], RANGE[4][2])) do bass_c[#bass_c + 1] = n end
  end
  local lt = scale_pc()[7]
  local s_c, a_c, t_c = cands(ch, RANGE[1][1], RANGE[1][2]), cands(ch, RANGE[2][1], RANGE[2][2]), cands(ch, RANGE[3][1], RANGE[3][2])
  for _, b in ipairs(bass_c) do
    for _, t in ipairs(t_c) do
      if t > b and t - b <= 19 then
        for _, a in ipairs(a_c) do
          if a > t and a - t <= 12 then
            for _, s in ipairs(s_c) do
              if s > a and s - a <= 12 then
                local v = { s, a, t, b }
                local cost = 0
                for i = 1, 4 do cost = cost + math.abs(v[i] - voices[i]) * (i == 4 and 0.5 or 1) end
                -- every chord tone present; the third must be there
                local has = {}
                for i = 1, 4 do has[v[i] % 12] = true end
                if not has[ch[2]] then cost = cost + 30 end
                if not has[ch[3]] then cost = cost + 4 end
                if b % 12 ~= ch[1] then cost = cost + 3 end
                -- never double the leading tone
                local n_lt = 0
                for i = 1, 4 do if v[i] % 12 == lt then n_lt = n_lt + 1 end end
                if n_lt > 1 then cost = cost + 20 end
                for i = 1, 3 do
                  for j = i + 1, 4 do
                    if motion_bad(voices[i], v[i], voices[j], v[j]) then cost = cost + 25 end
                  end
                end
                -- a singable soprano: favour steps, leaps cost more
                local leap = math.abs(s - voices[1])
                if leap > 4 then cost = cost + leap end
                cost = cost + math.random() * 1.5
                if cost < bc then best, bc = v, cost end
              end
            end
          end
        end
      end
    end
  end
  return best or voices
end

local function choose_next()
  count = count + 1
  local into = cadence_at - count
  if forced and into > 2 then
    local f = forced
    forced = nil
    return f
  end
  if into == 2 then return (math.random() < 0.5) and 2 or 4 end
  if into == 1 then return 5 end
  if into <= 0 then
    cadence_at = count + 8
    return (math.random() < 0.8) and 1 or 6 -- authentic, or deceptive
  end
  local opts = NEXT[degree]
  return opts[math.random(#opts)]
end

local function sing()
  local arp = params:get("arp") == 1
  for i = 4, 1, -1 do
    engine.pan(util.linlin(1, 4, 0.5, -0.5, i))
    engine.pw(0.35 + i * 0.05)
    engine.cutoff(params:get("bright") * (i == 4 and 0.6 or 1))
    engine.release(params:get("hold"))
    engine.amp(i == 1 and 0.2 or 0.15)
    engine.hz(MusicUtil.note_num_to_freq(voices[i]))
    if arp then clock.sleep(0.05) end
  end
  sung = 15
end

function init()
  params:add_separator("CHORALE")
  params:add_number("key", "key", 48, 60, 50, function(p) return MusicUtil.note_num_to_name(p:get(), false) end)
  params:add_option("mode", "mode", { "major", "minor" }, 1)
  params:add_number("bpm", "chords/min", 10, 120, 40)
  params:add_control("bright", "brightness", controlspec.new(300, 5000, 'exp', 0, 1500, 'hz'))
  params:add_control("hold", "hold", controlspec.new(0.5, 6, 'lin', 0, 2.6, 's'))
  params:add_option("arp", "arpeggiate", { "on", "off" }, 1)
  params:default()
  engine.gain(0.6)
  math.randomseed(os.time())
  voices = voice(1)
  local m = midi.connect()
  m.event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      -- pads C D E F G A B C pick degrees I..vii
      local pcs = { 0, 2, 4, 5, 7, 9, 11 }
      for d = 1, 7 do if msg.note % 12 == pcs[d] then forced = d end end
    end
  end
  clock.run(function()
    while true do
      table.insert(hist, { v = { table.unpack(voices) }, d = degree })
      while #hist > 10 do table.remove(hist, 1) end
      sing()
      clock.sleep(60 / params:get("bpm"))
      degree = choose_next()
      voices = voice(degree)
    end
  end)
  local frame = metro.init(function()
    sung = math.max(0, sung - 1)
    redraw()
  end, 1 / 20)
  frame:start()
end

function enc(n, d)
  if n == 2 then params:delta("bpm", d)
  elseif n == 3 then params:delta("bright", d) end
  redraw()
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then
    cadence_at = count + 8
    forced = ({ 4, 6, 2, 3 })[math.random(4)]
  elseif n == 3 then
    cadence_at = count + 3
  end
  redraw()
end

function redraw()
  screen.clear()
  local W = 9
  -- four lines, one per voice, across the recent chords
  for i = 1, 4 do
    screen.level(({ 15, 9, 6, 11 })[i])
    for k, h in ipairs(hist) do
      local x = 4 + (k - 1) * W
      local y = util.linlin(38, 82, 60, 12, h.v[i])
      if k == 1 then screen.move(x, y) else screen.line(x, y) end
      screen.line(x + W - 2, y)
    end
    screen.stroke()
  end
  -- the chord names under the newest few
  screen.level(15)
  screen.move(127, 8)
  screen.text_right("chorale")
  local last = hist[#hist]
  if last then
    screen.level(math.max(4, sung))
    screen.move(127, 30)
    screen.text_right(NAMES[last.d])
    screen.level(4)
    screen.move(127, 40)
    screen.text_right(MusicUtil.note_num_to_name(last.v[1], true))
    screen.move(127, 48)
    screen.text_right(MusicUtil.note_num_to_name(last.v[4], true))
  end
  screen.level(3)
  screen.move(127, 62)
  screen.text_right((cadence_at - count) .. " to cad.")
  screen.update()
end
