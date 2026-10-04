-- freezer
-- a Portamax norns script
--
-- a shimmering texture of soft
-- notes is always being written to
-- tape. K3 freezes the last couple
-- of seconds into a held loop and
-- the texture thins to a lone line
-- over it. K3 again thaws it.
--
-- E2 frozen level   E3 texture rate
-- K2 new harmony   K3 freeze / thaw
-- (params: loop length, tone)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local CHORDS = { { 0, "major 7" }, { 9, "minor 7" }, { 5, "major 7" }, { 2, "minor 7" }, { 7, "sus4" } }
local chord = {}
local frozen = false
local crystals = {}
local melt = 0

local function new_harmony()
  local c = CHORDS[math.random(#CHORDS)]
  chord = MusicUtil.generate_chord(60 + c[1] - (c[1] > 6 and 12 or 0), c[2], math.random(0, 2))
  local extra = {}
  for _, n in ipairs(chord) do extra[#extra + 1] = n + 12 end
  for _, n in ipairs(extra) do chord[#chord + 1] = n end
end

local function setup()
  local len = params:get("length")
  audio.level_eng_cut(1)
  softcut.buffer_clear()
  softcut.enable(1, 1)
  softcut.buffer(1, 1)
  softcut.loop(1, 1)
  softcut.loop_start(1, 1)
  softcut.loop_end(1, 1 + len)
  softcut.position(1, 1)
  softcut.fade_time(1, 0.15)
  softcut.level_slew_time(1, 0.6)
  softcut.level(1, 0)
  softcut.level_input_cut(1, 1, 1.0)
  softcut.level_input_cut(2, 1, 1.0)
  softcut.rec_level(1, 1)
  softcut.pre_level(1, 0.3)
  softcut.play(1, 1)
  softcut.rec(1, 1)
  softcut.filter_dry(1, 0.2)
  softcut.filter_lp(1, 0.8)
  softcut.filter_fc(1, 3500)
end

local function set_frozen(f)
  frozen = f
  softcut.rec(1, f and 0 or 1)
  softcut.level(1, f and params:get("hold") or 0)
end

function init()
  params:add_separator("FREEZER")
  params:add_control("hold", "frozen level", controlspec.new(0, 1.2, 'lin', 0, 0.9, ''))
  params:set_action("hold", function(x) if frozen then softcut.level(1, x) end end)
  params:add_control("rate", "texture rate", controlspec.new(1, 8, 'lin', 1, 4, '/beat'))
  params:add_control("length", "loop length", controlspec.new(0.5, 4, 'lin', 0.25, 2, 's'))
  params:set_action("length", function(x) softcut.loop_end(1, 1 + x) end)
  params:add_control("tone", "tone", controlspec.new(500, 6000, 'exp', 0, 2800, 'hz'))
  params:set_action("tone", function(x) engine.cutoff(x) end)
  params:default()
  engine.release(1.8)
  engine.pw(0.5)
  math.randomseed(os.time())
  new_harmony()
  setup()
  clock.run(function()
    while true do
      local r = params:get("rate")
      clock.sync(1 / r)
      -- frozen: only an occasional low line sings over the loop
      if not frozen or math.random() < 0.25 / r * 2 then
        local n = chord[math.random(#chord)] - (frozen and 12 or 0)
        engine.amp(frozen and 0.22 or 0.12 + math.random() * 0.08)
        engine.pan(math.random() * 1.6 - 0.8)
        engine.hz(MusicUtil.note_num_to_freq(n))
        table.insert(crystals, { x = math.random(8, 120), y = math.random(14, 50), l = 12 })
      end
    end
  end)
  clock.run(function()
    while true do
      clock.sleep(1 / 15)
      melt = util.clamp(melt + (frozen and 0.08 or -0.08), 0, 1)
      for i = #crystals, 1, -1 do
        crystals[i].l = crystals[i].l - (frozen and 0.1 or 0.6)
        if crystals[i].l <= 0 then table.remove(crystals, i) end
      end
      redraw()
    end
  end)
end

function enc(n, d)
  if n == 2 then params:delta("hold", d)
  elseif n == 3 then params:delta("rate", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then new_harmony()
  elseif n == 3 then set_frozen(not frozen) end
end

function redraw()
  screen.clear()
  for _, c in ipairs(crystals) do
    screen.level(math.max(1, math.floor(c.l)))
    local s = 2 + math.floor(melt * 3)
    screen.move(c.x - s, c.y)
    screen.line(c.x + s, c.y)
    screen.move(c.x, c.y - s)
    screen.line(c.x, c.y + s)
    screen.stroke()
  end
  if melt > 0 then
    screen.level(math.floor(2 + melt * 6))
    screen.rect(2, 12, 124, 42)
    screen.stroke()
  end
  screen.level(15)
  screen.move(0, 8)
  screen.text(frozen and "freezer (frozen)" or "freezer")
  screen.level(4)
  screen.move(0, 62)
  screen.text("hold " .. string.format("%.2f", params:get("hold")))
  screen.move(127, 62)
  screen.text_right(params:get("rate") .. "/beat")
  screen.update()
end

function cleanup()
  softcut.rec(1, 0)
end
