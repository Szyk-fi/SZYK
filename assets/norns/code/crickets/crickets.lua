-- crickets
-- a Portamax norns script
--
-- a field of crickets on a summer
-- night. like real snowy tree
-- crickets they chirp faster as it
-- warms (Dolbear's law: chirps per
-- minute = 4 x degrees F - 160).
-- each cricket has its own pitch.
--
-- E2 temperature   E3 crickets
-- K2 new chorus   K3 hush
-- pads: a frog answers
-- (params: scale, root, pulses)

engine.name = 'PolyPerc'

local MusicUtil = require "musicutil"

local crickets = {}
local scale = {}
local hushed = false
local frog = 0

local function build_scale()
  scale = MusicUtil.generate_scale_of_length(params:get("root"), params:get("scale"), 12)
end

local function cpm() return math.max(20, 4 * params:get("temp") - 160) end

local function chorus()
  for i = 1, 8 do
    crickets[i] = {
      x = 8 + math.random() * 112, y = 40 + math.random() * 12,
      deg = math.random(2, 8),
      phase = math.random() * 0.6, -- seconds until its next chirp
      drift = 0.92 + math.random() * 0.16, glow = 0,
    }
  end
  crickets[1].phase = 0.05
end

local function chirp(c)
  c.glow = 12
  clock.run(function()
    for p = 1, params:get("pulses") do
      engine.pan((c.x - 64) / 64 * 0.9)
      engine.pw(0.12)
      engine.release(0.05)
      engine.hz(MusicUtil.note_num_to_freq(scale[c.deg] + 24))
      clock.sleep(0.035)
    end
  end)
end

function init()
  local names = {}
  for i = 1, #MusicUtil.SCALES do names[i] = string.lower(MusicUtil.SCALES[i].name) end
  params:add_separator("CRICKETS")
  params:add_option("scale", "scale", names, 11)
  params:set_action("scale", build_scale)
  params:add_number("root", "root", 48, 72, 57, function(p) return MusicUtil.note_num_to_name(p:get(), true) end)
  params:set_action("root", build_scale)
  params:add_number("temp", "temperature", 50, 100, 72, function(p) return p:get() .. " F" end)
  params:add_number("count", "crickets", 1, 8, 4)
  params:add_number("pulses", "pulses", 1, 5, 3)
  params:add_control("bright", "brightness", controlspec.new(1000, 10000, 'exp', 0, 4500, 'hz'))
  params:set_action("bright", function(x) engine.cutoff(x) end)
  params:default()
  engine.amp(0.12)
  math.randomseed(os.time())
  build_scale()
  chorus()
  midi.connect().event = function(data)
    local msg = midi.to_msg(data)
    if msg.type == "note_on" then
      engine.pw(0.5)
      engine.release(0.4)
      engine.hz(MusicUtil.note_num_to_freq(MusicUtil.snap_note_to_array(msg.note - 12, scale) - 12))
      frog = 10
    end
  end
  local dt = 1 / 30
  metro.init(function()
    local period = 60 / cpm()
    for i, c in ipairs(crickets) do
      c.glow = math.max(0, c.glow - 1)
      if i <= params:get("count") and not hushed then
        c.phase = c.phase - dt
        if c.phase <= 0 then
          c.phase = c.phase + period * c.drift * (0.95 + math.random() * 0.1)
          chirp(c)
        end
      end
    end
    frog = math.max(0, frog - 1)
    redraw()
  end, dt):start()
end

function enc(n, d)
  if n == 2 then params:delta("temp", d)
  elseif n == 3 then params:delta("count", d) end
end

function key(n, z)
  if z == 0 then return end
  if n == 2 then chorus()
  elseif n == 3 then hushed = not hushed end
end

function redraw()
  screen.clear()
  -- moon
  screen.level(6)
  screen.circle(108, 18, 6)
  screen.fill()
  -- thermometer
  local tf = util.linlin(50, 100, 0, 34, params:get("temp"))
  screen.level(3)
  screen.rect(4, 14, 4, 36)
  screen.stroke()
  screen.level(12)
  screen.rect(5, 49 - tf, 2, tf)
  screen.fill()
  for i, c in ipairs(crickets) do
    if i <= params:get("count") then
      screen.level(math.max(3, c.glow))
      screen.rect(c.x - 2, c.y, 4, 2)
      screen.fill()
      if c.glow > 6 then
        screen.move(c.x - 3, c.y - 2)
        screen.line(c.x - 1, c.y - 5)
        screen.move(c.x + 3, c.y - 2)
        screen.line(c.x + 1, c.y - 5)
        screen.stroke()
      end
    end
  end
  screen.level(2)
  for x = 12, 127, 4 do screen.move(x, 54) screen.line(x + 2, 47 + (x * 5) % 5) screen.stroke() end
  if frog > 0 then screen.level(frog) screen.circle(64, 52, 3) screen.fill() end
  screen.level(15)
  screen.move(0, 8)
  screen.text(hushed and "crickets (hush)" or "crickets")
  screen.level(4)
  screen.move(0, 62)
  screen.text(params:string("temp") .. "  " .. cpm() .. "/min")
  screen.move(127, 62)
  screen.text_right(params:get("count") .. " crickets")
  screen.update()
end
