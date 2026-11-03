// The Acrobat form API, as far as form scripts need it, written in plain JavaScript.
//
// This file and the scripts of the document are the only code that runs. The engine has no host
// functions: nothing here can read or write files, reach the network, open links or windows. Calls
// that would (submitForm, launchURL, mailDoc, print, …) are recorded as "blocked" and do nothing.
//
// Input: __INPUT (a JSON string) with fields, calculation order, document scripts and the event.
// Output: the value of the last expression, JSON.stringify(__out).

var __in = JSON.parse(__INPUT);
var __out = { changes: {}, alerts: [], errors: [], rejected: null, blocked: [] };
var __fields = {};
var __names = [];
(function () {
  for (var i = 0; i < __in.fields.length; i++) {
    var f = __in.fields[i];
    __fields[f.name] = {
      name: f.name, type: f.kind, value: f.value, orig: f.value, dv: f.default_value,
      readonly: f.read_only, required: f.required, hidden: f.hidden,
      orig_readonly: f.read_only, orig_required: f.required, orig_hidden: f.hidden,
      exports: f.export_values, options: f.options, scripts: f.scripts
    };
    __names.push(f.name);
  }
})();

function __block(what) {
  if (__out.blocked.indexOf(what) < 0) __out.blocked.push(what);
}

function __isNumeric(v) {
  return typeof v === "string" && v !== "" && /^\s*[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?\s*$/.test(v);
}

function __str(v) {
  if (v === null || v === undefined) return "";
  if (typeof v === "number") {
    if (!isFinite(v)) return "";
    // Avoid 0.30000000000000004 style tails, as Acrobat shows them rounded.
    var r = Math.round(v * 1e10) / 1e10;
    return String(r);
  }
  return String(v);
}

// ---- Field ------------------------------------------------------------------------------

function Field(st) { this.__st = st; }
(function () {
  var P = Field.prototype;
  function prop(name, get, set) {
    Object.defineProperty(P, name, { get: get, set: set || function () {}, configurable: true });
  }
  prop("name", function () { return this.__st.name; });
  prop("type", function () { return this.__st.type; });
  prop("value", function () {
    var st = this.__st;
    if ((st.type === "text" || st.type === "combobox") && __isNumeric(st.value)) return Number(st.value);
    return st.value;
  }, function (v) { __setValue(this.__st, __str(v)); });
  prop("valueAsString", function () { return __str(this.__st.value); }, function (v) { __setValue(this.__st, __str(v)); });
  prop("rawValue", function () { return this.__st.value; }, function (v) { __setValue(this.__st, __str(v)); });
  prop("defaultValue", function () { return this.__st.dv; });
  prop("readonly", function () { return this.__st.readonly; }, function (v) { this.__st.readonly = !!v; });
  prop("required", function () { return this.__st.required; }, function (v) { this.__st.required = !!v; });
  prop("hidden", function () { return this.__st.hidden; }, function (v) { this.__st.hidden = !!v; });
  prop("display", function () { return this.__st.hidden ? 1 : 0; }, function (v) { this.__st.hidden = (v === 1 || v === 3); });
  prop("exportValues", function () { return this.__st.exports.slice(); });
  prop("numItems", function () { return this.__st.options.length; });
  prop("currentValueIndices", function () {
    for (var i = 0; i < this.__st.options.length; i++) if (this.__st.options[i][0] === this.__st.value) return i;
    return -1;
  }, function (i) {
    var o = this.__st.options[typeof i === "number" ? i : (i && i[0])];
    if (o) __setValue(this.__st, o[0]);
  });
  P.getItemAt = function (i, bExport) {
    var o = this.__st.options[i];
    if (!o) return "";
    return bExport === false ? o[1] : o[0];
  };
  P.isBoxChecked = function (i) { return this.__st.value !== "Off" && this.__st.value === this.__st.exports[i || 0]; };
  P.isDefaultChecked = function () { return false; };
  P.checkThisBox = function (i, b) {
    var e = this.__st.exports[i || 0] || "Yes";
    __setValue(this.__st, (b === undefined || b) ? e : "Off");
  };
  P.getArray = function () {
    var out = [], p = this.__st.name + ".";
    for (var i = 0; i < __names.length; i++) if (__names[i].indexOf(p) === 0) out.push(new Field(__fields[__names[i]]));
    return out;
  };
  // Look and focus: accepted and ignored.
  var ignored = ["textColor", "fillColor", "strokeColor", "borderColor", "textSize", "textFont",
    "borderStyle", "lineWidth", "alignment", "charLimit", "multiline", "doNotScroll", "comb",
    "userName", "print", "rotation", "buttonPosition", "style"];
  for (var k = 0; k < ignored.length; k++) {
    (function (n) {
      var key = "__" + n;
      prop(n, function () { return this[key]; }, function (v) { this[key] = v; });
    })(ignored[k]);
  }
  P.setFocus = function () {};
  P.setAction = function () { __block("setAction"); };
  P.setItems = function () { __block("setItems"); };
  P.buttonSetCaption = function () {};
  P.buttonImportIcon = function () { __block("buttonImportIcon"); return 1; };
  P.signatureSign = function () { __block("signatureSign"); return false; };
})();

function __setValue(st, v) {
  if (st.type === "checkbox" || st.type === "radiobutton") {
    if (v === "" || v === "0" || v === "false") v = "Off";
  }
  st.value = v;
}

// ---- The document is the global object ---------------------------------------------------

var __global = this;
function getField(n) {
  n = __str(n);
  if (__fields[n]) return new Field(__fields[n]);
  // A group name: a field object whose getArray() gives the members.
  for (var i = 0; i < __names.length; i++) {
    if (__names[i].indexOf(n + ".") === 0) {
      return new Field({ name: n, type: "text", value: "", orig: "", dv: "", readonly: false,
        required: false, hidden: false, exports: [], options: [], scripts: {}, group: true });
    }
  }
  return null;
}
function getNthFieldName(i) { return __names[i] || null; }
Object.defineProperty(__global, "numFields", { get: function () { return __names.length; } });
var numPages = __in.pages;
var pageNum = 0;
var documentFileName = __in.file_name;
var path = "/" + __in.file_name;
var dirty = true;
var info = { Title: "", Author: "", Subject: "", Keywords: "" };
function resetForm(names) {
  var list = names ? (typeof names === "string" ? [names] : names) : __names;
  for (var i = 0; i < list.length; i++) {
    var f = getField(list[i]);
    if (!f) continue;
    var targets = f.__st.group ? f.getArray() : [f];
    for (var j = 0; j < targets.length; j++) {
      var st = targets[j].__st;
      __setValue(st, st.dv || ((st.type === "checkbox" || st.type === "radiobutton") ? "Off" : ""));
    }
  }
}
function calculateNow() { __calculate(null); }
function submitForm() { __block("submitForm"); }
function mailDoc() { __block("mailDoc"); }
function mailForm() { __block("mailForm"); }
function print() { __block("print"); }
function saveAs() { __block("saveAs"); }
function closeDoc() { __block("closeDoc"); }
function exportAsFDF() { __block("exportAsFDF"); }
function exportAsXFDF() { __block("exportAsXFDF"); }
function importAnFDF() { __block("importAnFDF"); }
function importAnXFDF() { __block("importAnXFDF"); }
function getURL() { __block("getURL"); }
function gotoNamedDest() {}
function addField() { __block("addField"); return null; }
function removeField() { __block("removeField"); }

var app = {
  viewerType: "Exchange-Pro", viewerVariation: "Full", viewerVersion: 11, formsVersion: 11,
  platform: "WIN", language: "ENU", calculate: true, runtimeHighlight: true,
  alert: function (c) {
    var msg = (c && typeof c === "object") ? c.cMsg : c;
    __out.alerts.push(__str(msg));
    return 1;
  },
  beep: function () {},
  response: function () { return null; },
  launchURL: function () { __block("launchURL"); },
  openDoc: function () { __block("openDoc"); return null; },
  execMenuItem: function () { __block("execMenuItem"); },
  setTimeOut: function () { __block("setTimeOut"); return {}; },
  setInterval: function () { __block("setInterval"); return {}; },
  clearTimeOut: function () {},
  clearInterval: function () {},
  mailMsg: function () { __block("mailMsg"); },
  focusRect: function () {}
};
var console = { println: function () {}, show: function () {}, hide: function () {}, clear: function () {} };
var display = { visible: 0, hidden: 1, noPrint: 2, noView: 3 };
var color = {
  transparent: ["T"], black: ["G", 0], white: ["G", 1], red: ["RGB", 1, 0, 0], green: ["RGB", 0, 1, 0],
  blue: ["RGB", 0, 0, 1], cyan: ["CMYK", 1, 0, 0, 0], magenta: ["CMYK", 0, 1, 0, 0],
  yellow: ["CMYK", 0, 0, 1, 0], dkGray: ["G", 0.25], gray: ["G", 0.5], ltGray: ["G", 0.75],
  equal: function (a, b) { return String(a) === String(b); }, convert: function (c) { return c; }
};
var border = { s: "solid", b: "beveled", d: "dashed", i: "inset", u: "underline" };
var style = { ch: "check", cr: "cross", di: "diamond", ci: "circle", st: "star", sq: "square" };
var position = { textOnly: 0, iconOnly: 1, iconTextV: 2, textIconV: 3, iconTextH: 4, textIconH: 5, overlay: 6 };
var scaleHow = { proportional: 0, anamorphic: 1 };
var scaleWhen = { always: 0, never: 1, tooBig: 2, tooSmall: 3 };
var highlight = { n: "none", i: "invert", p: "push", o: "outline" };
var event = null;

// ---- util ---------------------------------------------------------------------------------

var __months = ["January", "February", "March", "April", "May", "June", "July", "August",
  "September", "October", "November", "December"];
var __days = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
function __pad(n, w) { n = String(n); while (n.length < w) n = "0" + n; return n; }

function __groupThousands(intPart, sep) {
  var out = "";
  while (intPart.length > 3) {
    out = sep + intPart.slice(-3) + out;
    intPart = intPart.slice(0, -3);
  }
  return intPart + out;
}

var util = {
  printf: function (fmt) {
    var args = arguments, ai = 1;
    return __str(fmt).replace(/%([,0-4]*)?(\+)?(\d+)?(?:\.(\d+))?([dfsx%])/g, function (m, sepFlag, plus, width, prec, conv) {
      if (conv === "%") return "%";
      var v = args[ai++];
      if (conv === "s") return __str(v);
      if (conv === "x") return (Math.floor(Number(v)) >>> 0).toString(16);
      var n = Number(v);
      if (!isFinite(n)) n = 0;
      var s;
      if (conv === "d") s = String(Math.trunc ? Math.trunc(n) : (n < 0 ? Math.ceil(n) : Math.floor(n)));
      else s = n.toFixed(prec === undefined ? 6 : Number(prec));
      var sepStyle = sepFlag ? Number(sepFlag.replace(",", "") || 0) : -1;
      if (sepFlag && sepFlag.indexOf(",") >= 0) {
        var neg = s.charAt(0) === "-";
        if (neg) s = s.slice(1);
        var parts = s.split(".");
        var seps = [[",", "."], ["", "."], [".", ","], ["", ","], ["'", "."]][sepStyle] || [",", "."];
        s = __groupThousands(parts[0], seps[0]) + (parts.length > 1 ? seps[1] + parts[1] : "");
        if (neg) s = "-" + s;
      }
      if (plus && n >= 0) s = "+" + s;
      return s;
    });
  },
  printd: function (fmt, d) {
    if (!(d instanceof Date) || isNaN(d.getTime())) return "";
    if (typeof fmt === "number") fmt = ["yyyy-mm-dd", "yy/mm/dd", "yyyy/mm/dd HH:MM:ss"][fmt] || "yyyy-mm-dd";
    var h = d.getHours();
    var map = {
      yyyy: String(d.getFullYear()), yy: __pad(d.getFullYear() % 100, 2),
      mmmm: __months[d.getMonth()], mmm: __months[d.getMonth()].slice(0, 3),
      mm: __pad(d.getMonth() + 1, 2), m: String(d.getMonth() + 1),
      dddd: __days[d.getDay()], ddd: __days[d.getDay()].slice(0, 3),
      dd: __pad(d.getDate(), 2), d: String(d.getDate()),
      HH: __pad(h, 2), H: String(h), hh: __pad(h % 12 || 12, 2), h: String(h % 12 || 12),
      MM: __pad(d.getMinutes(), 2), M: String(d.getMinutes()),
      ss: __pad(d.getSeconds(), 2), s: String(d.getSeconds()),
      tt: h < 12 ? "am" : "pm", t: h < 12 ? "a" : "p"
    };
    return __str(fmt).replace(/yyyy|yy|mmmm|mmm|mm|m|dddd|ddd|dd|d|HH|H|hh|h|MM|M|ss|s|tt|t/g, function (k) { return map[k]; });
  },
  scand: function (fmt, s) {
    s = __str(s).trim ? __str(s).trim() : __str(s);
    if (s === "") return null;
    var tokens = __str(fmt).match(/yyyy|yy|mmmm|mmm|mm|m|dd|d|HH|H|hh|h|MM|M|ss|s|tt|./g) || [];
    var nums = s.match(/\d+|[A-Za-z]+/g) || [];
    var y = 1970, mo = 0, da = 1, hh = 0, mi = 0, se = 0, pm = null, ni = 0;
    for (var i = 0; i < tokens.length; i++) {
      var t = tokens[i];
      if (!/^[ymdHhMst]/.test(t) || t.length === 1 && !/[ymdHhMs]/.test(t)) continue;
      var v = nums[ni++];
      if (v === undefined) break;
      if (t === "yyyy") y = Number(v);
      else if (t === "yy") y = 2000 + Number(v) - (Number(v) > 50 ? 100 : 0);
      else if (t === "mmmm" || t === "mmm") {
        var found = -1;
        for (var k = 0; k < 12; k++) if (__months[k].toLowerCase().indexOf(v.toLowerCase().slice(0, 3)) === 0) found = k;
        if (found < 0) return null;
        mo = found;
      } else if (t === "mm" || t === "m") mo = Number(v) - 1;
      else if (t === "dd" || t === "d") da = Number(v);
      else if (t === "HH" || t === "H" || t === "hh" || t === "h") hh = Number(v);
      else if (t === "MM" || t === "M") mi = Number(v);
      else if (t === "ss" || t === "s") se = Number(v);
      else if (t === "tt") pm = /^p/i.test(v);
      if (isNaN(Number(v)) && t !== "mmmm" && t !== "mmm" && t !== "tt") return null;
    }
    if (pm === true && hh < 12) hh += 12;
    if (pm === false && hh === 12) hh = 0;
    var d = new Date(y, mo, da, hh, mi, se);
    if (d.getMonth() !== mo || d.getDate() !== da) return null;
    return d;
  },
  printx: function (fmt, s) {
    s = __str(s);
    var out = "", si = 0;
    fmt = __str(fmt);
    for (var i = 0; i < fmt.length; i++) {
      var c = fmt.charAt(i);
      if (c === "9") { while (si < s.length && !/\d/.test(s.charAt(si))) si++; out += s.charAt(si++) || ""; }
      else if (c === "?") out += s.charAt(si++) || "";
      else if (c === "*") { out += s.slice(si); si = s.length; }
      else out += c;
    }
    return out;
  },
  crackURL: function () { __block("crackURL"); return {}; }
};

// ---- AF functions (Acrobat's built-in form functions) -----------------------------------

function AFMakeNumber(v) {
  if (typeof v === "number") return isFinite(v) ? v : null;
  var s = __str(v).replace(/[\s '€$£¥%]/g, "").replace(/[A-Za-z]/g, "");
  if (s === "") return null;
  var neg = /^\(.*\)$/.test(s);
  s = s.replace(/[()]/g, "");
  var lc = s.lastIndexOf(","), ld = s.lastIndexOf(".");
  if (lc >= 0 && ld >= 0) {
    if (lc > ld) s = s.replace(/\./g, "").replace(",", ".");
    else s = s.replace(/,/g, "");
  } else if (lc >= 0) {
    // "1,5" is a decimal comma; "1,500" with exactly three digits after it is a thousands comma.
    s = /^-?\d{1,3}(,\d{3})+$/.test(s) ? s.replace(/,/g, "") : s.replace(",", ".");
  }
  var n = parseFloat(s);
  if (isNaN(n)) return null;
  return neg ? -n : n;
}

function __formatNumber(n, nDec, sepStyle) {
  var neg = n < 0;
  var s = Math.abs(n).toFixed(nDec);
  var parts = s.split(".");
  var seps = [[",", "."], ["", "."], [".", ","], ["", ","], ["'", "."]][sepStyle || 0] || [",", "."];
  return { neg: neg, text: __groupThousands(parts[0], seps[0]) + (parts.length > 1 ? seps[1] + parts[1] : "") };
}

function AFNumber_Format(nDec, sepStyle, negStyle, currStyle, strCurrency, bCurrencyPrepend) {
  if (event.value === "" || event.value === null) { event.value = ""; return; }
  var n = AFMakeNumber(event.value);
  if (n === null) { event.value = ""; return; }
  var f = __formatNumber(n, nDec, sepStyle);
  var t = f.text;
  var cur = strCurrency || "";
  if (cur) t = bCurrencyPrepend !== false ? cur + t : t + cur;
  if (f.neg && Number(f.text.replace(/[^\d]/g, "")) !== 0) {
    t = (negStyle === 2 || negStyle === 3) ? "(" + t + ")" : "-" + t;
  }
  event.value = t;
}

function AFNumber_Keystroke(nDec, sepStyle) {
  if (!event.willCommit) return;
  var v = __str(event.value);
  if (v === "") return;
  if (AFMakeNumber(v) === null) {
    event.rc = false;
    app.alert("The value entered does not match the format of the field [ " + event.target.name + " ]");
  }
}

function AFPercent_Format(nDec, sepStyle, bPercentPrepend) {
  if (event.value === "" || event.value === null) { event.value = ""; return; }
  var n = AFMakeNumber(event.value);
  if (n === null) { event.value = ""; return; }
  var f = __formatNumber(n * 100, nDec, sepStyle);
  var t = (f.neg ? "-" : "") + f.text;
  event.value = bPercentPrepend ? "%" + t : t + "%";
}
function AFPercent_Keystroke(nDec, sepStyle) { AFNumber_Keystroke(nDec, sepStyle); }

var __dateFormats = ["m/d", "m/d/yy", "mm/dd/yy", "mm/yy", "d-mmm", "d-mmm-yy", "dd-mmm-yy",
  "yy-mm-dd", "mmm-yy", "mmmm-yy", "mmm d, yyyy", "mmmm d, yyyy", "m/d/yy h:MM tt", "m/d/yy HH:MM"];
var __timeFormats = ["HH:MM", "h:MM tt", "HH:MM:ss", "h:MM:ss tt"];

function AFParseDateEx(s, fmt) {
  var d = util.scand(fmt, s);
  if (d) return d;
  // Three numbers: year first when it has four digits, else day and month in the order of the format.
  var n = __str(s).match(/\d+/g);
  if (n && n.length >= 3) {
    var y, m, day;
    if (n[0].length === 4) { y = Number(n[0]); m = Number(n[1]); day = Number(n[2]); }
    else {
      var f = __str(fmt);
      var dayFirst = f.indexOf("d") >= 0 && f.indexOf("m") >= 0 ? f.indexOf("d") < f.indexOf("m") : false;
      y = Number(n[2]); if (y < 100) y += 2000;
      if (dayFirst) { day = Number(n[0]); m = Number(n[1]); } else { m = Number(n[0]); day = Number(n[1]); }
    }
    var r = new Date(y, m - 1, day);
    if (r.getMonth() === m - 1 && r.getDate() === day) return r;
    return null;
  }
  var t = Date.parse(__str(s));
  return isNaN(t) ? null : new Date(t);
}

function AFDate_FormatEx(cFormat) {
  if (event.value === "" || event.value === null) { event.value = ""; return; }
  var d = AFParseDateEx(event.value, cFormat);
  event.value = d ? util.printd(cFormat, d) : "";
}
function AFDate_Format(n) { AFDate_FormatEx(__dateFormats[n] || "m/d/yy"); }
function AFDate_KeystrokeEx(cFormat) {
  if (!event.willCommit || __str(event.value) === "") return;
  if (!AFParseDateEx(event.value, cFormat)) {
    event.rc = false;
    app.alert("Invalid date/time: please ensure that the date/time exists. Field [ " + event.target.name + " ] should match format " + cFormat);
  }
}
function AFDate_Keystroke(n) { AFDate_KeystrokeEx(__dateFormats[n] || "m/d/yy"); }
function AFTime_FormatEx(cFormat) { AFDate_FormatEx(cFormat); }
function AFTime_Format(n) { AFDate_FormatEx(__timeFormats[n] || "HH:MM"); }
function AFTime_KeystrokeEx(cFormat) { AFDate_KeystrokeEx(cFormat); }
function AFTime_Keystroke(n) { AFDate_KeystrokeEx(__timeFormats[n] || "HH:MM"); }

function AFSpecial_Format(psf) {
  var d = __str(event.value).replace(/\D/g, "");
  if (d === "") { event.value = ""; return; }
  var masks = ["99999", "99999-9999", "(999) 999-9999", "999-99-9999"];
  event.value = util.printx(masks[psf] || "*", d);
}
function AFSpecial_Keystroke(psf) {
  if (!event.willCommit || __str(event.value) === "") return;
  var d = __str(event.value).replace(/\D/g, "");
  var need = [[5], [9], [10, 7], [9]][psf] || [];
  if (need.length && need.indexOf(d.length) < 0) {
    event.rc = false;
    app.alert("The value entered does not match the format of the field [ " + event.target.name + " ]");
  }
}
function AFSpecial_KeystrokeEx(mask) {}

function AFMergeChange(e) { return __str(e.value) + __str(e.change); }

function AFSimple_Calculate(cFunction, cFields) {
  var names = typeof cFields === "string" ? cFields.split(",") : cFields;
  var vals = [];
  for (var i = 0; i < names.length; i++) {
    var f = getField(__str(names[i]).replace(/^\s+|\s+$/g, ""));
    if (!f) continue;
    var members = f.__st.group ? f.getArray() : [f];
    for (var j = 0; j < members.length; j++) {
      var st = members[j].__st;
      var v = st.type === "checkbox" || st.type === "radiobutton"
        ? (st.value === "Off" ? 0 : AFMakeNumber(st.value))
        : AFMakeNumber(st.value);
      vals.push(v === null ? 0 : v);
    }
  }
  var r = 0;
  switch (__str(cFunction).toUpperCase()) {
    case "SUM": for (var a = 0; a < vals.length; a++) r += vals[a]; break;
    case "PRD": r = vals.length ? 1 : 0; for (var b = 0; b < vals.length; b++) r *= vals[b]; break;
    case "AVG": for (var c = 0; c < vals.length; c++) r += vals[c]; r = vals.length ? r / vals.length : 0; break;
    case "MIN": r = vals.length ? Math.min.apply(null, vals) : 0; break;
    case "MAX": r = vals.length ? Math.max.apply(null, vals) : 0; break;
  }
  event.value = r;
}

function AFRange_Validate(bGreaterThan, nGreaterThan, bLessThan, nLessThan) {
  if (__str(event.value) === "") return;
  var v = AFMakeNumber(event.value);
  if (v === null) return;
  var msg = "";
  if (bGreaterThan && bLessThan) {
    if (v < nGreaterThan || v > nLessThan) msg = "The value entered must be greater than or equal to " + nGreaterThan + " and less than or equal to " + nLessThan + ".";
  } else if (bGreaterThan) {
    if (v < nGreaterThan) msg = "The value entered must be greater than or equal to " + nGreaterThan + ".";
  } else if (bLessThan) {
    if (v > nLessThan) msg = "The value entered must be less than or equal to " + nLessThan + ".";
  }
  if (msg) { app.alert(msg); event.rc = false; }
}

// ---- Running ----------------------------------------------------------------------------

function __event(name, target, value, source) {
  return {
    name: name, type: "Field", value: value, rc: true, target: new Field(target),
    targetName: target.name, source: source ? new Field(source) : new Field(target),
    change: "", changeEx: "", willCommit: true, selStart: 0, selEnd: 0, commitKey: 2,
    modifier: false, shift: false, fieldFull: false, keyDown: false
  };
}

function __exec(__src) { return eval(__src); }

function __run(src, ev, what) {
  event = ev;
  try {
    __exec.call(__global, src);
  } catch (e) {
    __out.errors.push(what + ": " + e);
  }
  event = null;
  return ev;
}

function __valueOf(st) {
  if ((st.type === "text" || st.type === "combobox") && __isNumeric(st.value)) return Number(st.value);
  return st.value;
}

function __calculate(source) {
  for (var i = 0; i < __in.order.length; i++) {
    var st = __fields[__in.order[i]];
    if (!st || !st.scripts.calculate) continue;
    var ev = __run(st.scripts.calculate, __event("Calculate", st, __valueOf(st), source), st.name + " (calculate)");
    if (ev.rc !== false) __setValue(st, __str(ev.value));
  }
}

function __formatAll() {
  for (var i = 0; i < __names.length; i++) {
    var st = __fields[__names[i]];
    if (!st.scripts.format) continue;
    var ev = __run(st.scripts.format, __event("Format", st, __valueOf(st)), st.name + " (format)");
    var c = __out.changes[st.name] || (__out.changes[st.name] = {});
    c.display = __str(ev.value);
  }
}

function __main() {
  for (var d = 0; d < __in.document.length; d++) {
    try { (0, eval)(__in.document[d]); } catch (e) { __out.errors.push("document script: " + e); }
  }
  var e = __in.event;
  if (e.kind === "commit") {
    var st = __fields[e.field];
    if (!st) return;
    var v = e.value;
    if (st.scripts.keystroke) {
      var k = __run(st.scripts.keystroke, __event("Keystroke", st, v), st.name + " (keystroke)");
      if (k.rc === false) { __out.rejected = st.name; return; }
      v = __str(k.value);
    }
    if (st.scripts.validate) {
      var va = __run(st.scripts.validate, __event("Validate", st, __isNumeric(v) ? Number(v) : v), st.name + " (validate)");
      if (va.rc === false) { __out.rejected = st.name; return; }
      v = __str(va.value);
    }
    __setValue(st, v);
    if (app.calculate) __calculate(st);
  } else if (e.kind === "click") {
    var b = __fields[e.field];
    if (b && b.scripts.action) __run(b.scripts.action, __event("Mouse Up", b, __valueOf(b)), b.name + " (action)");
    if (app.calculate) __calculate(b);
  } else {
    __calculate(null);
  }
  __formatAll();
}

__main();
for (var __i = 0; __i < __names.length; __i++) {
  var __st = __fields[__names[__i]];
  var __c = __out.changes[__st.name] || {};
  if (__st.value !== __st.orig) __c.value = __st.value;
  if (__st.hidden !== __st.orig_hidden) __c.hidden = __st.hidden;
  if (__st.readonly !== __st.orig_readonly) __c.read_only = __st.readonly;
  if (__st.required !== __st.orig_required) __c.required = __st.required;
  var __any = false;
  for (var __k in __c) { __any = true; break; }
  if (__any) __out.changes[__st.name] = __c;
}
JSON.stringify(__out);
