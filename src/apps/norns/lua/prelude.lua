-- Portamax's norns environment: the globals a norns script expects,
-- written for Portamax from the documented norns Lua API. The host
-- (host.rs) provides `_px` (engine/softcut/audio commands, time, print)
-- and the `screen` table before this runs, and calls
-- `_px_tick(now)` about a thousand times a second.

local _px = _px

-------------------------------------------------------------------- print
print = function(...)
  local t = {}
  for i = 1, select("#", ...) do t[#t + 1] = tostring(select(i, ...)) end
  _px.print(table.concat(t, "\t"))
end

--------------------------------------------------------------------- util
util = {}
function util.clamp(n, lo, hi) return math.min(math.max(n, lo), hi) end
function util.linlin(slo, shi, dlo, dhi, f)
  if f <= slo then return dlo elseif f >= shi then return dhi end
  return (f - slo) / (shi - slo) * (dhi - dlo) + dlo
end
function util.linexp(slo, shi, dlo, dhi, f)
  if f <= slo then return dlo elseif f >= shi then return dhi end
  return dlo * (dhi / dlo) ^ ((f - slo) / (shi - slo))
end
function util.explin(slo, shi, dlo, dhi, f)
  if f <= slo then return dlo elseif f >= shi then return dhi end
  return math.log(f / slo) / math.log(shi / slo) * (dhi - dlo) + dlo
end
function util.expexp(slo, shi, dlo, dhi, f)
  if f <= slo then return dlo elseif f >= shi then return dhi end
  return dlo * (dhi / dlo) ^ (math.log(f / slo) / math.log(shi / slo))
end
function util.round(n, q)
  q = q or 1
  return math.floor(n / q + 0.5) * q
end
function util.round_up(n, q)
  q = q or 1
  return math.ceil(n / q) * q
end
function util.wrap(n, lo, hi)
  local r = hi - lo + 1
  return ((n - lo) % r) + lo
end
function util.wrap_max(n, lo, hi)
  local r = hi - lo
  return ((n - lo) % r) + lo
end
function util.time() return _px.now() end
function util.acronym(s)
  local out = ""
  for w in string.gmatch(s, "%S+") do out = out .. string.sub(w, 1, 1) end
  return out
end
function util.file_exists(p)
  local f = io.open(p, "r")
  if f then f:close() return true end
  return false
end
function util.s_to_hms(s)
  s = math.floor(s)
  return string.format("%d:%02d:%02d", s // 3600, (s // 60) % 60, s % 60)
end
function util.dbamp(db) return 10 ^ (db / 20) end
function util.ampdb(a) return 20 * math.log(a, 10) end
function util.degs_to_rads(d) return d * math.pi / 180 end
function util.trim_string_to_width(s, w)
  local chars = math.floor(w / 5)
  if #s > chars then return string.sub(s, 1, chars) end
  return s
end
function util.os_capture() return "" end
function util.scandir() return {} end
function util.make_dir() end

---------------------------------------------------------------------- tab
tab = {}
function tab.print(t) for k, v in pairs(t) do print(tostring(k) .. "\t" .. tostring(v)) end end
function tab.count(t) local n = 0 for _ in pairs(t) do n = n + 1 end return n end
function tab.contains(t, x) for _, v in pairs(t) do if v == x then return true end end return false end
function tab.key(t, x) for k, v in pairs(t) do if v == x then return k end end return nil end
function tab.invert(t) local o = {} for k, v in pairs(t) do o[v] = k end return o end
function tab.sort(t) local keys = {} for k in pairs(t) do keys[#keys + 1] = k end table.sort(keys) return keys end
function tab.split(s, sep)
  local o = {}
  for part in string.gmatch(s, "([^" .. sep .. "]+)") do o[#o + 1] = part end
  return o
end
function tab.save() end
function tab.load() return nil end

-------------------------------------------------------------- controlspec
controlspec = {}
controlspec.__index = controlspec

function controlspec.new(minval, maxval, warp, step, default, units, quantum, wrap)
  local s = setmetatable({}, controlspec)
  s.minval = minval or 0
  s.maxval = maxval or 1
  s.warp = warp or "lin"
  if type(s.warp) == "string" then s.warp = string.lower(s.warp) end
  s.step = step or 0
  s.default = default or s.minval
  s.units = units or ""
  s.quantum = quantum or 0.01
  s.wrap = wrap or false
  return s
end

function controlspec.def(t)
  return controlspec.new(t.min, t.max, t.warp, t.step, t.default, t.units, t.quantum, t.wrap)
end

function controlspec:map(x)
  x = util.clamp(x, 0, 1)
  local v
  if self.warp == "exp" and self.minval > 0 and self.maxval > 0 then
    v = self.minval * (self.maxval / self.minval) ^ x
  elseif self.warp == "db" then
    v = util.linlin(0, 1, self.minval, self.maxval, x)
  else
    v = self.minval + (self.maxval - self.minval) * x
  end
  if self.step > 0 then v = util.round(v, self.step) end
  return v
end

function controlspec:unmap(v)
  local lo, hi = math.min(self.minval, self.maxval), math.max(self.minval, self.maxval)
  v = util.clamp(v, lo, hi)
  if self.maxval == self.minval then return 0 end
  if self.warp == "exp" and self.minval > 0 and self.maxval > 0 then
    return math.log(v / self.minval) / math.log(self.maxval / self.minval)
  end
  return (v - self.minval) / (self.maxval - self.minval)
end

function controlspec:constrain(v)
  return util.clamp(v, math.min(self.minval, self.maxval), math.max(self.minval, self.maxval))
end

function controlspec:copy()
  return controlspec.new(self.minval, self.maxval, self.warp, self.step, self.default, self.units, self.quantum, self.wrap)
end

controlspec.UNIPOLAR = controlspec.new(0, 1, "lin", 0, 0, "")
controlspec.BIPOLAR = controlspec.new(-1, 1, "lin", 0, 0, "")
controlspec.FREQ = controlspec.new(20, 20000, "exp", 0, 440, "Hz")
controlspec.LOFREQ = controlspec.new(0.1, 100, "exp", 0, 6, "Hz")
controlspec.MIDFREQ = controlspec.new(25, 4200, "exp", 0, 440, "Hz")
controlspec.WIDEFREQ = controlspec.new(0.1, 20000, "exp", 0, 440, "Hz")
controlspec.PHASE = controlspec.new(0, math.pi, "lin", 0, 0, "")
controlspec.RQ = controlspec.new(0.001, 2, "exp", 0, 0.707, "")
controlspec.AMP = controlspec.new(0, 1, "lin", 0, 0, "")
controlspec.BOOSTCUT = controlspec.new(-20, 20, "lin", 0, 0, "dB")
controlspec.DB = controlspec.new(-60, 0, "db", 0, 0, "dB")
controlspec.PAN = controlspec.new(-1, 1, "lin", 0, 0, "")
controlspec.DELAY = controlspec.new(0.0001, 1, "exp", 0, 0.3, "secs")

------------------------------------------------------------------- params
local Param = {}
Param.__index = Param

local function fmt_number(x)
  if math.type and math.type(x) == "integer" then return tostring(x) end
  local a = math.abs(x)
  if x == math.floor(x) and a < 1e9 then return string.format("%d", x) end
  if a >= 100 then return string.format("%.0f", x) end
  if a >= 10 then return string.format("%.1f", x) end
  return string.format("%.2f", x)
end

function Param:get()
  if self.t == "control" or self.t == "taper" then return self.spec:map(self.raw) end
  return self.value
end

function Param:get_raw()
  if self.t == "control" or self.t == "taper" then return self.raw end
  if self.t == "number" then return util.linlin(self.min, self.max, 0, 1, self.value) end
  if self.t == "option" then return util.linlin(1, #self.options, 0, 1, self.value) end
  return self.value
end

function Param:bang()
  if self.action and self.t ~= "separator" and self.t ~= "group" and self.t ~= "trigger" then
    self.action(self:get())
  end
end

function Param:set(v, silent)
  local t = self.t
  if t == "control" or t == "taper" then
    self.raw = self.spec:unmap(v)
  elseif t == "number" then
    v = math.floor(v + 0.5)
    if self.wrap then v = util.wrap(v, self.min, self.max) else v = util.clamp(v, self.min, self.max) end
    self.value = v
  elseif t == "option" then
    self.value = util.clamp(math.floor(v + 0.5), 1, #self.options)
  elseif t == "binary" then
    self.value = (v ~= 0 and v ~= false) and 1 or 0
  elseif t == "trigger" then
    if self.action and not silent then self.action(1) end
    return
  elseif t == "text" then
    self.value = tostring(v)
  else
    return
  end
  if not silent then self:bang() end
end

function Param:set_raw(r, silent)
  if self.t == "control" or self.t == "taper" then
    self.raw = util.clamp(r, 0, 1)
    if not silent then self:bang() end
  elseif self.t == "number" then
    self:set(util.linlin(0, 1, self.min, self.max, r), silent)
  end
end

function Param:delta(d)
  local t = self.t
  if t == "control" or t == "taper" then
    local q = self.spec.quantum or 0.01
    local raw = self.raw + d * q
    if self.spec.wrap then raw = raw % 1 else raw = util.clamp(raw, 0, 1) end
    self.raw = raw
    self:bang()
  elseif t == "number" then
    self:set(self.value + d)
  elseif t == "option" then
    self:set(self.value + d)
  elseif t == "binary" then
    if d ~= 0 then self:set(d > 0 and 1 or 0) end
  elseif t == "trigger" then
    if d > 0 then self:set(1) end
  end
end

function Param:string()
  local t = self.t
  if self.formatter then return self.formatter(self) end
  if t == "control" or t == "taper" then
    local u = self.spec.units or ""
    return fmt_number(self:get()) .. (u ~= "" and (" " .. u) or "")
  elseif t == "number" then
    return tostring(self.value)
  elseif t == "option" then
    return tostring(self.options[self.value])
  elseif t == "binary" then
    return self.value == 1 and "on" or "off"
  elseif t == "trigger" then
    return ""
  elseif t == "text" then
    return self.value
  end
  return ""
end

local ParamSet = {}
ParamSet.__index = ParamSet

function ParamSet.new(id, name)
  return setmetatable({ id = id or "params", name = name or "", params = {}, lookup = {}, count = 0, hidden = {} }, ParamSet)
end

function ParamSet:_add(p)
  p.__set = self
  setmetatable(p, Param)
  table.insert(self.params, p)
  self.count = #self.params
  if p.id then self.lookup[p.id] = #self.params end
  return p
end

function ParamSet:add(a)
  local t = a.type or "number"
  local p = { t = t, id = a.id, name = a.name or a.id or "", action = a.action, formatter = a.formatter }
  if t == "control" or t == "taper" then
    local spec = a.controlspec
    if not spec and t == "taper" then
      spec = controlspec.new(a.min or 0, a.max or 1, (a.k and a.k ~= 0) and "exp" or "lin", 0, a.default or a.min or 0, a.units or "")
    end
    spec = spec or controlspec.UNIPOLAR
    p.spec = spec
    p.raw = spec:unmap(spec.default)
    p.default = p.raw
  elseif t == "number" then
    p.min = a.min or -2147483648
    p.max = a.max or 2147483647
    p.wrap = a.wrap
    p.value = a.default or 0
    p.default = p.value
  elseif t == "option" then
    p.options = a.options or {}
    p.value = a.default or 1
    p.default = p.value
  elseif t == "binary" then
    p.value = a.default or 0
    p.behavior = a.behavior or "toggle"
    p.default = p.value
  elseif t == "text" then
    p.value = a.text or a.default or ""
  elseif t == "group" then
    p.n = a.n or 0
  end
  return self:_add(p)
end

function ParamSet:add_number(id, name, min, max, default, formatter, wrap)
  return self:add { type = "number", id = id, name = name, min = min, max = max, default = default, formatter = formatter, wrap = wrap }
end
function ParamSet:add_option(id, name, options, default)
  return self:add { type = "option", id = id, name = name, options = options, default = default }
end
function ParamSet:add_control(id, name, spec, formatter)
  return self:add { type = "control", id = id, name = name, controlspec = spec, formatter = formatter }
end
function ParamSet:add_taper(id, name, min, max, default, k, units)
  return self:add { type = "taper", id = id, name = name, min = min, max = max, default = default, k = k, units = units }
end
function ParamSet:add_trigger(id, name)
  return self:add { type = "trigger", id = id, name = name }
end
function ParamSet:add_binary(id, name, behavior, default)
  return self:add { type = "binary", id = id, name = name, behavior = behavior, default = default }
end
function ParamSet:add_text(id, name, text)
  return self:add { type = "text", id = id, name = name, text = text }
end
function ParamSet:add_file(id, name, path)
  return self:add { type = "text", id = id, name = name, text = path or "" }
end
function ParamSet:add_separator(id, name)
  return self:add { type = "separator", id = (name and id) or nil, name = name or id or "" }
end
function ParamSet:add_group(id, name, n)
  -- (name, n) or (id, name, n)
  if type(name) == "number" then n = name name = id id = nil end
  return self:add { type = "group", id = id, name = name, n = n }
end

function ParamSet:lookup_param(id)
  local i = type(id) == "number" and id or self.lookup[id]
  local p = i and self.params[i]
  if not p then error("invalid param: " .. tostring(id), 3) end
  return p
end

function ParamSet:get(id) return self:lookup_param(id):get() end
function ParamSet:get_raw(id) return self:lookup_param(id):get_raw() end
function ParamSet:set(id, v, silent) self:lookup_param(id):set(v, silent) end
function ParamSet:set_raw(id, v, silent) self:lookup_param(id):set_raw(v, silent) end
function ParamSet:delta(id, d) self:lookup_param(id):delta(d) end
function ParamSet:string(id) return self:lookup_param(id):string() end
function ParamSet:set_action(id, f) self:lookup_param(id).action = f end
function ParamSet:get_id(i) return self.params[i] and self.params[i].id end
function ParamSet:t(id) return self:lookup_param(id).t end
function ParamSet:visible() return true end
function ParamSet:hide(id) self.hidden[id] = true end
function ParamSet:show(id) self.hidden[id] = nil end
function ParamSet:bang() for _, p in ipairs(self.params) do p:bang() end end
function ParamSet:default() self:bang() end
function ParamSet:read() end
function ParamSet:write() end
function ParamSet:clear() self.params = {} self.lookup = {} self.count = 0 end

paramset = ParamSet
params = ParamSet.new()

-- What the Params page shows: { kind, name, value } per row.
function _px_params_list()
  local out = {}
  for i, p in ipairs(params.params) do
    local kind = p.t
    out[#out + 1] = { kind, p.name or "", (kind == "separator" or kind == "group") and "" or p:string(), i }
  end
  return out
end

function _px_params_delta(i, d)
  local p = params.params[i]
  if p then p:delta(d) end
end

-------------------------------------------------------------------- clock
clock = { threads = {}, next_id = 1, beats = 0, last = nil, transport = {} }

local function tempo()
  local ok, v = pcall(function() return params:get("clock_tempo") end)
  if ok and v then return v end
  return 120
end

function clock.get_tempo() return tempo() end
function clock.get_beats() return clock.beats end
function clock.get_beat_sec(x)
  -- callable as clock.get_beat_sec() or clock:get_beat_sec()
  return 60 / tempo()
end
function clock.set_source() end
function clock.internal_set_tempo(t) pcall(function() params:set("clock_tempo", t) end) end
clock.internal = { set_tempo = clock.internal_set_tempo, start = function() end, stop = function() end }

local function schedule(id, ok, kind, a, b)
  local th = clock.threads[id]
  if not th then return end
  if not ok then
    clock.threads[id] = nil
    _px.error(tostring(kind))
    return
  end
  if coroutine.status(th.co) == "dead" then
    clock.threads[id] = nil
    return
  end
  if kind == "sleep" then
    th.wake_time = _px.now() + (a or 0)
    th.wake_beat = nil
  elseif kind == "sync" then
    local div = (a and a > 0) and a or 1
    local off = b or 0
    local now = clock.beats
    local n = math.floor((now - off) / div + 1e-9) + 1
    th.wake_beat = n * div + off
    th.wake_time = nil
  else
    -- a bare yield: run again next tick
    th.wake_time = _px.now()
    th.wake_beat = nil
  end
end

function clock.run(f, ...)
  local id = clock.next_id
  clock.next_id = id + 1
  local co = coroutine.create(f)
  clock.threads[id] = { co = co }
  schedule(id, coroutine.resume(co, ...))
  return id
end

function clock.cancel(id) clock.threads[id] = nil end
function clock.sleep(s) return coroutine.yield("sleep", s) end
function clock.sync(b, off) return coroutine.yield("sync", b, off) end
function clock.cleanup() clock.threads = {} end

------------------------------------------------------------------- metro
metro = {}
metro.__index = metro

function metro.init(a, time, count)
  local m = setmetatable({ time = 1, count = -1, event = nil, is_running = false, id = nil }, metro)
  if type(a) == "table" then
    m.event, m.time, m.count = a.event, a.time or 1, a.count or -1
  else
    m.event, m.time, m.count = a, time or 1, count or -1
  end
  return m
end

function metro:start(time, count, stage)
  if type(time) == "table" then time, count, stage = time.time, time.count, time.stage end
  if time then self.time = time end
  if count then self.count = count end
  self:stop()
  self.is_running = true
  local me = self
  self.id = clock.run(function()
    local s = stage or 1
    while me.count < 0 or s <= me.count do
      clock.sleep(me.time)
      if not me.is_running then return end
      if me.event then me.event(s) end
      s = s + 1
    end
    me.is_running = false
  end)
end

function metro:stop()
  if self.id then clock.cancel(self.id) end
  self.id = nil
  self.is_running = false
end

function metro.free_all() end

------------------------------------------------------------- tick (host)
function _px_tick(now)
  local last = clock.last or now
  clock.last = now
  clock.beats = clock.beats + (now - last) * tempo() / 60
  local due = {}
  for id, th in pairs(clock.threads) do
    if (th.wake_time and th.wake_time <= now) or (th.wake_beat and th.wake_beat <= clock.beats) then
      due[#due + 1] = id
    end
  end
  table.sort(due)
  for _, id in ipairs(due) do
    local th = clock.threads[id]
    if th then schedule(id, coroutine.resume(th.co)) end
  end
end

-------------------------------------------------------- engine / softcut
local function forwarder(send)
  return setmetatable({}, {
    __index = function(_, k)
      return function(...)
        local args = { ... }
        -- allow both softcut.x(...) and softcut:x(...)
        if type(args[1]) == "table" then table.remove(args, 1) end
        send(k, table.unpack(args))
      end
    end,
  })
end

engine = setmetatable({ name = nil }, {
  __index = function(t, k)
    if k == "list_commands" then return function() print("PolyPerc: hz amp pw release cutoff gain pan") end end
    return function(...) _px.engine(k, ...) end
  end,
})

softcut = forwarder(_px.softcut)
audio = forwarder(_px.audio)
poll = { set = function() return { start = function() end, stop = function() end, update = function() end } end }
osc = { send = function() end }
keyboard = { code = function() end, state = {} }
hid = { connect = function() return {} end, vports = {} }

---------------------------------------------------------------- midi etc
midi = { vports = {}, devices = {} }
local function midi_device(i)
  local d = { name = (i == 1) and "portamax pads" or "none", port = i, event = nil }
  function d:send(data) _px.midi_out(i, data) end
  function d:note_on(n, v, ch) self:send({ 0x90 + (ch or 1) - 1, n, v or 100 }) end
  function d:note_off(n, v, ch) self:send({ 0x80 + (ch or 1) - 1, n, v or 0 }) end
  function d:cc(c, v, ch) self:send({ 0xB0 + (ch or 1) - 1, c, v }) end
  function d:pitchbend(v, ch) end
  function d:program_change(v, ch) self:send({ 0xC0 + (ch or 1) - 1, v }) end
  function d:start() self:send({ 0xFA }) end
  function d:stop() self:send({ 0xFC }) end
  function d:continue() self:send({ 0xFB }) end
  function d:clock() self:send({ 0xF8 }) end
  return d
end
for i = 1, 16 do midi.vports[i] = midi_device(i) end
function midi.connect(n) return midi.vports[n or 1] or midi.vports[1] end
function midi.to_msg(data)
  local s, a, b = data[1] or 0, data[2] or 0, data[3] or 0
  local t, ch = s & 0xF0, (s & 0x0F) + 1
  if s == 0xFA then return { type = "start" } end
  if s == 0xFC then return { type = "stop" } end
  if s == 0xFB then return { type = "continue" } end
  if s == 0xF8 then return { type = "clock" } end
  if t == 0x90 and b > 0 then return { type = "note_on", note = a, vel = b, ch = ch } end
  if t == 0x80 or (t == 0x90 and b == 0) then return { type = "note_off", note = a, vel = b, ch = ch } end
  if t == 0xB0 then return { type = "cc", cc = a, val = b, ch = ch } end
  if t == 0xE0 then return { type = "pitchbend", val = a + (b << 7), ch = ch } end
  return { type = "other" }
end
function midi.to_data(m)
  local ch = (m.ch or 1) - 1
  if m.type == "note_on" then return { 0x90 + ch, m.note, m.vel or 100 } end
  if m.type == "note_off" then return { 0x80 + ch, m.note, m.vel or 0 } end
  if m.type == "cc" then return { 0xB0 + ch, m.cc, m.val } end
  return {}
end

-- No grid or arc is attached: these behave like norns' unattached
-- virtual ports (drawing goes nowhere, no key events arrive).
local function device(rows, cols)
  local d = { rows = rows, cols = cols, name = "none", device = nil }
  function d:all() end
  function d:led() end
  function d:segment() end
  function d:refresh() end
  function d:rotation() end
  function d:intensity() end
  return d
end
grid = { vports = {} }
function grid.connect() return device(8, 16) end
arc = { vports = {} }
function arc.connect() return device(0, 0) end

-- crow: absorbs anything (no crow is attached)
local function sink()
  local s
  s = setmetatable({}, {
    __index = function() return s end,
    __newindex = function() end,
    __call = function() return s end,
  })
  return s
end
crow = sink()

norns = {
  enc = { sens = function() end, accel = function() end },
  state = {},
  script = { load = function() end, clear = function() end },
  version = { update = "portamax" },
  crow = sink(),
}
_norns = norns

----------------------------------------------------------------- include
function include(name)
  for _, dir in ipairs(_px.search_dirs) do
    local f = dir .. "/" .. name .. ".lua"
    if util.file_exists(f) then return dofile(f) end
  end
  error("include: can't find " .. name, 2)
end

----------------------------------------------- screen calls with no effect
for _, k in ipairs({ "aa", "font_face", "font_size", "line_cap", "line_join", "miter_limit", "blend_mode",
  "save", "restore", "translate", "rotate", "ping", "display_png", "poke", "sleep", "reset" }) do
  if not screen[k] then screen[k] = function() end end
end
screen.peek = screen.peek or function() return "" end
screen.text_extents = function(s) return #tostring(s) * 5, 8 end
screen.font_face_count = 1
screen.font_face_names = { "default" }

-------------------------------------------------------- system params
function _px_system_params()
  params:add_separator("CLOCK")
  params:add_number("clock_tempo", "tempo", 1, 300, 120)
end
