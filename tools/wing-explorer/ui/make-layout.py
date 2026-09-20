#!/usr/bin/env python3
"""Author the panel geometry for the grandMA3 onPC command wing.

MA's `hardware_configurations.xml` says what every index *is* and nothing at all about
where it sits — `AnimationPos` is a boot-animation ordering, not a coordinate — so the
drawing has to be authored. The placements below follow the real panel: the encoder
bank and ten executor faders on the left, the two motorised playback faders in the
middle, the five big encoders across the top right, the command block under them and
the datawheel down the right-hand edge.

The output holds geometry and indices only. Names come from the control map at runtime,
so nothing of MA's ends up checked in.

    python3 make-layout.py > layout.json
    python3 make-layout.py /path/to/hardware_configurations.xml > layout.json

Reads MA's file with a real XML parser. A regex over it is not good enough: two of the
LED definitions are commented out, and a regex happily matches inside a comment.
"""
import json, os, sys, xml.etree.ElementTree as ET

MODULE = "grandMA3 onPC command wing"


def find_ma3_xml():
    base = os.path.expanduser("~/MALightingTechnology")
    installs = sorted((d for d in os.listdir(base)
                       if d.startswith("gma3_") and d != "gma3_library"), reverse=True)
    for d in installs:
        p = os.path.join(base, d, "shared/resource/hardware_configurations.xml")
        if os.path.exists(p):
            return p
    sys.exit("no grandMA3 under ~/MALightingTechnology; pass the xml explicitly")


def load(path):
    """MA's table, in the shape this script wants: index -> attributes."""
    for cfg in ET.parse(path).getroot().iter("HardwareConfiguration"):
        if cfg.get("Name") != MODULE:
            continue
        def listed(parent, tag):
            block = cfg.find(parent)
            return {} if block is None else {
                i: dict(e.attrib) for i, e in enumerate(block.findall(tag))
            }
        return {
            "hardkeys": {i: a for i, a in listed("Hardkeys", "Hardkey").items() if a.get("Code")},
            "faders": listed("FaderDefinitions", "FaderDefinition"),
            "encoders": listed("EncoderDefinitions", "EncoderDefinition"),
        }
    sys.exit("no %r in %s" % (MODULE, path))

W, H = 1400, 700

KW, KH = 52, 30          # a command key

# What is printed on the key. MA's `Code` is the console's internal name and is not
# always the legend: the desk says Go+ where the table says DEF_GO. The panel is what
# an operator reads, so the panel wins — and this is the only place that opinion lives.
LEGEND = {
    "DEF_GO": "Go+",
    "DEF_GOBACK": "Go-",
    "DEF_PAUSE": "Pause",
    "PAGE_UP": "Page+",
    "PAGE_DOWN": "Page-",
    "UNDO": "Oops",
    "PREVIEW": "Prvw",
    "HIGHLIGHT": "Hilight",
    "SEQUENCE": "Sequ",
    "USER1": "U1",
    "USER2": "U2",
    "GOFAST": ">>>",
    "GOBACKFAST": "<<<",
    "SELFIX": "SelFix",
    "MA1": "MA",
    "MA2": "MA",
    # The keypad reads as a keypad. MA's codes are NUM7 and SLASH; the desk says 7
    # and /, and the panel is what an operator is looking at.
    "NUM0": "0", "NUM1": "1", "NUM2": "2", "NUM3": "3", "NUM4": "4",
    "NUM5": "5", "NUM6": "6", "NUM7": "7", "NUM8": "8", "NUM9": "9",
    "SLASH": "/", "PLUS": "+", "MINUS": "-", "DOT": ".",
}
GAP = 5
SMALL = 26               # an executor button


def grid(ox, oy, cols, codes, kw=KW, kh=KH, gap=GAP):
    """Lay `codes` out row-major. An empty string leaves a hole."""
    for i, code in enumerate(codes):
        if not code:
            continue
        yield code, ox + (i % cols) * (kw + gap), oy + (i // cols) * (kh + gap), kw, kh


def main():
    src = sys.argv[1] if len(sys.argv) > 1 else find_ma3_xml()
    m = load(src)

    by_code = {}
    for idx, c in m["hardkeys"].items():
        code = (c.get("Code") or "").strip()
        if code and code not in ("EXEC", "FADER"):
            by_code.setdefault(code, idx)

    items, groups = [], []

    def key(code, x, y, w=KW, h=KH, compact=False, legend=None):
        idx = by_code.get(code)
        if idx is None:
            return
        it = {"kind": "key", "index": idx, "x": x, "y": y, "w": w, "h": h}
        if legend is not None:
            it["legend"] = legend
        elif code in LEGEND:
            it["legend"] = LEGEND[code]
        if compact:
            it["compact"] = True
        items.append(it)

    def block(title, x, y, w, h):
        groups.append({"title": title, "x": x, "y": y, "w": w, "h": h})

    # ---------------------------------------------------------------- region 3
    # The command block. Left column of state keys, then the paired keys, then the
    # X keys, the playback row and the keypad.
    block("Command", 640, 252, 600, 388)

    for code, x, y, w, h in grid(655, 255, 1,
                                 ["HIGHLIGHT", "SOLO", "FREEZE", "PREVIEW", "BLIND",
                                  "USER1", "USER2"]):
        key(code, x, y, w, h)
    for code, x, y, w, h in grid(715, 255, 2, [
        "ON", "OFF", "MOVE", "COPY", "DELETE", "ALIGN", "STOMP", "HELP", "SELECT", "GOTO",
    ]):
        key(code, x, y, w, h)

    # Above X1-X8 sits an encoder each: those keys are executors 291-298, and MA's
    # table has an encoder for every one of them. Then the grand master, which is
    # encoder 6 (`SpecialExecutor="GrandKnob"`) with hardkey 46 as its push.
    enc_for = {c.get("ExecutorIndex"): i for i, c in
               m["encoders"].items() if c.get("ExecutorIndex")}
    for n in range(8):
        ex = str(291 + n)
        if ex in enc_for:
            items.append({"kind": "encoder", "index": enc_for[ex],
                          "x": 835 + n * 44 + 20, "y": 270, "r": 9})
    # X keys, two rows of eight, then the three that sit beside them.
    xs1 = ["X1", "X2", "X3", "X4", "X5", "X6", "X7", "X8"]
    xs2 = ["X9", "X10", "X11", "X12", "X13", "X14", "X15", "X16"]
    for code, x, y, w, h in grid(835, 290, 8, xs1, kw=40, kh=26, gap=4):
        key(code, x, y, w, h, compact=True)
    for code, x, y, w, h in grid(835, 322, 8, xs2, kw=40, kh=26, gap=4):
        key(code, x, y, w, h, compact=True)
    key("XKEYS", 1190, 290, 44, 26, compact=True)
    key("LIST", 1190, 322, 44, 26, compact=True)

    # The playback row under the X keys.
    for code, x, y, w, h in grid(835, 360, 3, ["PAUSE", "GOBACK", "GO"], kw=48, kh=28, gap=5):
        key(code, x, y, w, h, compact=True)
    # The wide key between Go+ and Learn. MA's table has MA1 and MA2; MA1 sits in the
    # keypad, so this is MA2. Bound rather than drawn as a hole, so that pressing it
    # is a real test: if no bit moves, that is a finding about the protocol and not a
    # gap in the drawing.
    key("MA2", 995, 360, 58, 28, compact=True)
    for code, x, y, w, h in grid(1065, 360, 3, ["LEARN", "GOBACKFAST", "GOFAST"],
                                 kw=44, kh=28, gap=5):
        key(code, x, y, w, h, compact=True)

    # The keypad half. Page+ and Page- sit at the bottom of the left column, level
    # with Edit and Update; Store is under Time, not under Assign.
    for code, x, y, w, h in grid(655, 425, 1, ["", "", "PAGE_UP", "PAGE_DOWN"]):
        key(code, x, y, w, h)
    for code, x, y, w, h in grid(715, 425, 3, [
        "FIXTURE", "CHANNEL", "GROUP",
        "PRESET", "SEQUENCE", "CUE",
        "EDIT", "ASSIGN", "TIME",
        "UPDATE", "", "STORE",
    ]):
        key(code, x, y, w, h)
    # 789+ / 456thru / 123- / 0.ifat / MA (space) / please
    for code, x, y, w, h in grid(890, 425, 4, [
        "NUM7", "NUM8", "NUM9", "PLUS",
        "NUM4", "NUM5", "NUM6", "THRU",
        "NUM1", "NUM2", "NUM3", "MINUS",
        "NUM0", "DOT", "IF", "AT",
        "MA1", "SLASH", "", "PLEASE",
    ]):
        key(code, x, y, w, h)
    key("ESC", 1130, 425, KW, KH)
    key("CLEAR", 1130, 495, KW, KH)
    key("FULL", 1130 + KW + GAP, 495, KW, KH)

    # ---------------------------------------------------------------- region 2
    # Five big encoders across the top, each with a press button below.
    block("Encoders", 640, 100, 600, 130)
    enc = m["encoders"]
    for n in range(1, 6):
        inside = next((i for i, c in enc.items() if c.get("Type") == "Inside%d" % n), None)
        outside = next((i for i, c in enc.items() if c.get("Type") == "Outside%d" % n), None)
        cx = 700 + (n - 1) * 110
        if inside is not None:
            # Pushing the knob is ENCODER_INSIDE<n>, so the knob itself is that key's
            # indicator - press it on the desk and it lights on the drawing.
            items.append({"kind": "encoder", "index": inside, "outer": outside,
                          "x": cx, "y": 160, "r": 38,
                          "press": by_code.get("ENCODER_INSIDE%d" % n)})
        # The small round button at the bottom-left corner is ENCODER_OUTSIDE<n>.
        idx = by_code.get("ENCODER_OUTSIDE%d" % n)
        if idx is not None:
            items.append({"kind": "key", "index": idx, "round": True,
                          "x": cx - 44, "y": 202, "w": 16, "h": 16})

    # ---------------------------------------------------------------- region 1
    # The two motorised playback faders, their navigation keys above and Go below.
    block("Playback", 540, 100, 90, 540)
    for code, x, y, w, h in grid(548, 112, 2, ["PREV", "NEXT", "SET", "UP", "SELFIX", "DOWN"],
                                 kw=36, kh=26, gap=4):
        key(code, x, y, w, h, compact=True)

    faders = m["faders"]
    touch_by_exec, exec_by_exec = {}, {}
    for idx, c in m["hardkeys"].items():
        ex = c.get("ExecutorIndex")
        if not ex:
            continue
        if c.get("Code") == "FADER":
            touch_by_exec[ex] = idx
        elif c.get("Code") == "EXEC":
            exec_by_exec[ex] = idx

    # The two crossfaders are the last two entries in MA's fader table.
    special = [i for i in sorted(faders) if faders[i].get("SpecialExecutor")]
    for n, i in enumerate(special[:2]):
        items.append({"kind": "fader", "index": i, "x": 556 + n * 40, "y": 200,
                      "w": 28, "h": 240, "touch": touch_by_exec.get(
                          faders[i].get("ExecutorIndex"))})
    for code, x, y, w, h in grid(552, 500, 1, ["DEF_PAUSE", "DEF_GOBACK", "DEF_GO"],
                                 kw=66, kh=34, gap=8):
        key(code, x, y, w, h)

    # ---------------------------------------------------------------- region 4
    # The right-hand column, in the order the panel has it: the grand master on top,
    # Menu under it, and the master wheel at the bottom.
    block("Master", 1250, 252, 130, 330)
    grand = next((i for i, c in enc.items()
                  if c.get("SpecialExecutor") == "GrandKnob"), None)
    if grand is not None:
        items.append({"kind": "encoder", "index": grand, "x": 1315, "y": 300, "r": 26,
                      "press": next((int(i) for i, c in m["hardkeys"].items()
                                     if c.get("SpecialExecutor") == "GrandKnob"), None)})
    key("MENU", 1293, 350, 44, 26, compact=True)
    wheel = next((i for i, c in enc.items() if c.get("Type") == "WheelMaster"), None)
    if wheel is not None:
        items.append({"kind": "encoder", "index": wheel, "x": 1315, "y": 460, "r": 34})

    # ------------------------------------------------------------- regions 5-8
    # The executor bank, seven rows deep, exactly as the panel has it:
    #
    #     encoders 401-410      the far encoder row
    #     buttons  401-410
    #     encoders 301-310      the near encoder row
    #     buttons  301-310
    #     faders   201-210
    #     buttons  201-210
    #     buttons  101-110
    #
    # MA's table has four rows of ten EXEC keys — 101, 201, 301 and 401 — and the
    # encoder-faders carry the 3xx and 4xx numbers, so every row here is placed by the
    # executor it belongs to. An earlier version walked the encoder table in index
    # order instead, which scattered them.
    block("Executors", 90, 100, 470, 460)

    enc_by_exec = {c.get("ExecutorIndex"): i for i, c in enc.items()
                   if c.get("Key") == "FADER" and c.get("ExecutorIndex")}
    fader_by_exec = {c.get("ExecutorIndex"): i for i, c in faders.items()
                     if c.get("ExecutorIndex")}

    def bank_x(col):
        """Ten across, in two banks of five, the way the panel splits them."""
        return 112 + col * 40 + (24 if col >= 5 else 0)

    def exec_row(hundred, y, kind):
        for col in range(10):
            ex = str(hundred + col + 1)
            x = bank_x(col)
            if kind == "encoder" and ex in enc_by_exec:
                items.append({"kind": "encoder", "index": enc_by_exec[ex],
                              "x": x + 13, "y": y + 11, "r": 11})
            elif kind == "key" and ex in exec_by_exec:
                items.append({"kind": "key", "index": exec_by_exec[ex],
                              "x": x, "y": y, "w": SMALL, "h": SMALL, "compact": True})
            elif kind == "fader" and ex in fader_by_exec:
                i = fader_by_exec[ex]
                items.append({"kind": "fader", "index": i, "x": x, "y": y,
                              "w": SMALL, "h": 140,
                              "touch": touch_by_exec.get(ex),
                              "exec": exec_by_exec.get(ex)})

    exec_row(400, 118, "encoder")
    exec_row(400, 150, "key")
    exec_row(300, 190, "encoder")
    exec_row(300, 222, "key")
    exec_row(200, 270, "fader")
    exec_row(200, 430, "key")
    exec_row(100, 466, "key")

    doc = {
        "width": W, "height": H,
        "note": "Authored geometry, not MA's: hardware_configurations.xml carries no "
                "coordinates. Laid out to follow the real panel — correct it against "
                "the desk and regenerate with ui/make-layout.py.",
        "groups": groups,
        "items": items,
    }
    json.dump(doc, sys.stdout, indent=1)
    print()


if __name__ == "__main__":
    main()
