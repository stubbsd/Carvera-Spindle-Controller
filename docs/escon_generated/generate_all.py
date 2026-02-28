#!/usr/bin/env python3
"""Generate clean recreations of all ESCON Studio screenshots in realistic Windows style."""

from PIL import Image, ImageDraw, ImageFont

ARIAL = "/System/Library/Fonts/Supplemental/Arial.ttf"
ARIAL_BOLD = "/System/Library/Fonts/Supplemental/Arial Bold.ttf"

# Standard dimensions
W, H = 800, 550

# Colors
C_TITLEBAR = "#0A246A"
C_BG = "#C0C0C0"
C_SUNKEN_DARK = "#404040"
C_SUNKEN_MID = "#808080"
C_SELECT = "#0A246A"
C_TABLE_HEADER = "#0A246A"


def load_fonts():
    return {
        "tree": ImageFont.truetype(ARIAL, 12),
        "tree_bold": ImageFont.truetype(ARIAL_BOLD, 12),
        "heading": ImageFont.truetype(ARIAL_BOLD, 16),
        "label": ImageFont.truetype(ARIAL, 12),
        "label_bold": ImageFont.truetype(ARIAL_BOLD, 12),
        "field": ImageFont.truetype(ARIAL, 12),
        "title": ImageFont.truetype(ARIAL_BOLD, 12),
        "small": ImageFont.truetype(ARIAL, 11),
        "tab": ImageFont.truetype(ARIAL, 11),
    }


# --- Full tree structure ---
FULL_TREE = [
    {"text": "Motor/Sensors", "level": 0, "expanded": True},
    {"text": "Motor", "level": 1},
    {"text": "Detection of Rotor Position", "level": 1},
    {"text": "Speed Sensor", "level": 1},
    {"text": "Controller", "level": 0, "expanded": True},
    {"text": "Mode of Operation", "level": 1},
    {"text": "Enable", "level": 1},
    {"text": "Set Value", "level": 1},
    {"text": "Current Limit", "level": 1},
    {"text": "Speed Ramp", "level": 1},
    {"text": "Minimal Speed", "level": 1},
    {"text": "Offset", "level": 1},
    {"text": "Internal Speed Regulator", "level": 1},
    {"text": "Regulation Tuning", "level": 1},
    {"text": "Inputs/Outputs", "level": 0, "expanded": True},
    {"text": "Digital Inputs and Outputs", "level": 1},
    {"text": "Analog Inputs", "level": 1},
    {"text": "Analog Outputs", "level": 1},
]


def draw_tree(draw, fonts, x0, y0, selected_item, tree_w=210):
    """Draw the properties tree with given item selected."""
    y = y0
    line_h = 20

    for item in FULL_TREE:
        indent = 20
        ix = x0 + item["level"] * indent
        is_selected = item["text"] == selected_item
        expanded = item.get("expanded", False)
        is_parent = item["level"] == 0

        if is_selected:
            draw.rectangle([x0 - 2, y - 1, x0 + tree_w, y + line_h - 3],
                           fill=C_SELECT)
            color = "white"
        else:
            color = "black"

        if is_parent:
            bx, by = ix - 2, y + 2
            draw.rectangle([bx, by, bx + 10, by + 10], outline=C_SUNKEN_MID, fill="white")
            sign = "-" if expanded else "+"
            draw.text((bx + 2, by - 2), sign, fill="black", font=fonts["tree"])
            ix += 14

        font = fonts["tree_bold"] if is_parent else fonts["tree"]
        draw.text((ix, y), item["text"], fill=color, font=font)
        y += line_h

    return y


def make_base(fonts, selected_item, height=None, tabs=None):
    """Create base image with title bar, menu, tree. Returns (img, draw, form_x, form_y, form_w)."""
    h = height or H
    img = Image.new("RGB", (W, h), C_BG)
    draw = ImageDraw.Draw(img)

    # Title bar
    draw.rectangle([0, 0, W, 24], fill=C_TITLEBAR)
    draw.text((6, 4), "MyProject2* - Motion Studio 1.17", fill="white", font=fonts["title"])
    for i, txt in enumerate(["_", "\u25a1", "X"]):
        bx = W - 60 + i * 20
        draw.rectangle([bx, 2, bx + 17, 20], fill=C_BG, outline=C_SUNKEN_MID)
        draw.text((bx + 4, 2), txt, fill="black", font=fonts["small"])

    # Menu bar
    draw.rectangle([0, 24, W, 44], fill=C_BG)
    draw.line([0, 44, W, 44], fill=C_SUNKEN_MID)
    for i, menu in enumerate(["Window", "Help"]):
        draw.text((10 + i * 70, 27), menu, fill="black", font=fonts["label"])

    # Tab bar if specified
    top_y = 44
    if tabs:
        tab_y = 44
        draw.rectangle([0, tab_y, W, tab_y + 24], fill=C_BG)
        draw.line([0, tab_y + 24, W, tab_y + 24], fill=C_SUNKEN_MID)
        tx = 10
        for tab_text in tabs:
            tw = draw.textlength(tab_text, font=fonts["tab"]) + 16
            # Tab shape
            draw.rectangle([tx, tab_y + 2, tx + tw, tab_y + 24], fill=C_BG, outline=C_SUNKEN_MID)
            draw.line([tx + 1, tab_y + 24, tx + tw - 1, tab_y + 24], fill=C_BG)
            draw.text((tx + 8, tab_y + 6), tab_text, fill="black", font=fonts["tab"])
            tx += tw + 4
        top_y = tab_y + 24

    # Properties label
    draw.text((10, top_y + 8), "Properties", fill="black", font=fonts["tree_bold"])

    # Left panel
    tree_x, tree_y = 8, top_y + 24
    tree_w_px, tree_h = 220, h - tree_y - 12
    draw.rectangle([tree_x, tree_y, tree_x + tree_w_px, tree_y + tree_h], fill="white")
    draw.line([tree_x, tree_y, tree_x + tree_w_px, tree_y], fill=C_SUNKEN_DARK)
    draw.line([tree_x, tree_y, tree_x, tree_y + tree_h], fill=C_SUNKEN_DARK)
    draw.line([tree_x + tree_w_px, tree_y, tree_x + tree_w_px, tree_y + tree_h], fill="white")
    draw.line([tree_x, tree_y + tree_h, tree_x + tree_w_px, tree_y + tree_h], fill="white")

    draw_tree(draw, fonts, tree_x + 8, tree_y + 8, selected_item)

    # Divider
    div_x = tree_x + tree_w_px + 8
    draw.line([div_x, top_y, div_x, h - 10], fill=C_SUNKEN_MID)

    form_x = div_x + 16
    form_y = top_y + 10
    form_w = W - form_x - 16

    return img, draw, form_x, form_y, form_w


def draw_dropdown(draw, fonts, x, y, text, w=220, h=22):
    """Draw a Windows-style dropdown. Returns bottom y."""
    draw.rectangle([x, y, x + w, y + h], fill="white", outline=C_SUNKEN_MID)
    draw.line([x, y, x + w, y], fill=C_SUNKEN_DARK)
    draw.line([x, y, x, y + h], fill=C_SUNKEN_DARK)
    btn_x = x + w - 18
    draw.rectangle([btn_x, y + 1, x + w - 1, y + h - 1], fill=C_BG, outline=C_SUNKEN_MID)
    ax = btn_x + 5
    ay = y + 9
    draw.polygon([(ax, ay), (ax + 8, ay), (ax + 4, ay + 5)], fill="black")
    draw.text((x + 4, y + 3), text, fill="black", font=fonts["field"])
    return y + h


def draw_sunken_field(draw, fonts, x, y, value, w=80, h=22):
    """Draw a sunken text input field."""
    draw.rectangle([x, y, x + w, y + h], fill="white", outline=C_SUNKEN_MID)
    draw.line([x, y, x + w, y], fill=C_SUNKEN_DARK)
    draw.line([x, y, x, y + h], fill=C_SUNKEN_DARK)
    draw.text((x + 4, y + 3), value, fill="black", font=fonts["field"])


def draw_radio(draw, fonts, x, y, label, selected=False):
    """Draw a radio button. Returns width consumed."""
    r = 6
    cx, cy = x + r, y + r + 2
    draw.ellipse([cx - r, cy - r, cx + r, cy + r], fill="white", outline=C_SUNKEN_MID)
    if selected:
        draw.ellipse([cx - 3, cy - 3, cx + 3, cy + 3], fill="black")
    tw = draw.textlength(label, font=fonts["label"])
    draw.text((x + r * 2 + 4, y), label, fill="black", font=fonts["label"])
    return r * 2 + 4 + tw + 16


def draw_table(draw, fonts, x, y, headers, rows, col_widths, highlight_row=None):
    """Draw a simple table. Returns bottom y."""
    row_h = 22
    # Header
    hx = x
    for i, hdr in enumerate(headers):
        draw.rectangle([hx, y, hx + col_widths[i], y + row_h], fill=C_TABLE_HEADER)
        draw.text((hx + 6, y + 4), hdr, fill="white", font=fonts["label_bold"])
        hx += col_widths[i]
    y += row_h

    # Rows
    for ri, row in enumerate(rows):
        hx = x
        is_highlight = highlight_row is not None and ri == highlight_row
        bg = C_SELECT if is_highlight else "white"
        fg = "white" if is_highlight else "black"
        for i, cell in enumerate(row):
            draw.rectangle([hx, y, hx + col_widths[i], y + row_h],
                           fill=bg, outline=C_SUNKEN_MID)
            draw.text((hx + 6, y + 4), cell, fill=fg, font=fonts["field"])
            hx += col_widths[i]
        y += row_h

    return y


# =========================================================================
# Screenshot generators
# =========================================================================

def gen_01_motor():
    """01: Motor - maxon EC motor, speed constant, pole pairs, current limits."""
    fonts = load_fonts()
    img, draw, fx, fy, fw = make_base(fonts, "Motor")

    y = fy
    draw.text((fx, y), "Motor", fill="black", font=fonts["heading"])
    y += 30

    # Section 1: Motor type
    draw.text((fx, y), "Please choose the motor type.", fill="black", font=fonts["label"])
    y += 22
    draw_dropdown(draw, fonts, fx, y, "maxon EC motor", w=220)
    y += 42

    # Section 2: Motor Data
    draw.text((fx, y), "Please enter the Motor Data Characteristics (see catalog motor data)",
              fill="black", font=fonts["label"])
    y += 22

    fields1 = [
        ("Speed Constant:", "242.2", "rpm/V"),
        ("Thermal Time Constant Winding:", "4.0", "s"),
        ("Number of Pole Pairs:", "4", ""),
    ]

    for label, value, unit in fields1:
        draw.text((fx + 10, y + 3), label, fill="black", font=fonts["label"])
        draw_sunken_field(draw, fonts, fx + 280, y, value)
        if unit:
            draw.text((fx + 366, y + 3), unit, fill="black", font=fonts["label"])
        y += 28

    y += 10

    # Section 3: Custom data
    draw.text((fx, y), "Please enter the custom data.", fill="black", font=fonts["label"])
    y += 22

    fields2 = [
        ("Max. Permissible Speed:", "12500.0", "rpm"),
        ("Nominal Current:", "4.1000", "A"),
        ("Max. Output Current Limit:", "8.2000", "A"),
    ]

    for label, value, unit in fields2:
        draw.text((fx + 10, y + 3), label, fill="black", font=fonts["label"])
        draw_sunken_field(draw, fonts, fx + 280, y, value)
        if unit:
            draw.text((fx + 366, y + 3), unit, fill="black", font=fonts["label"])
        y += 28

    img.save("docs/escon_generated/01_motor.png")
    print("Saved 01_motor.png")


def gen_02_rotor_position():
    """02: Detection of Rotor Position - Hall Sensors, Inverted polarity."""
    fonts = load_fonts()
    img, draw, fx, fy, fw = make_base(fonts, "Detection of Rotor Position")

    y = fy
    draw.text((fx, y), "Detection of Rotor Position", fill="black", font=fonts["heading"])
    y += 30

    draw.text((fx, y), "Please choose a sensor type.", fill="black", font=fonts["label"])
    y += 22
    draw_dropdown(draw, fonts, fx, y, "Digital Hall Sensors", w=240)
    y += 40

    draw.text((fx, y), "Hall Sensor Polarity:", fill="black", font=fonts["label"])
    y += 22
    rx = fx + 10
    rw = draw_radio(draw, fonts, rx, y, "normal", selected=False)
    draw_radio(draw, fonts, rx + rw + 10, y, "Inverted", selected=True)
    y += 30

    # Rotor position visualization
    draw.text((fx, y), "Rotor position:", fill="black", font=fonts["label"])
    y += 20

    # Draw commutation diagram - 6 steps shown as colored blocks
    block_w = 50
    block_h = 16
    labels = ["0\u00b0", "60\u00b0", "120\u00b0", "180\u00b0", "240\u00b0", "300\u00b0"]
    hall_labels = ["Hall 1", "Hall 2", "Hall 3"]
    # Hall sensor states per 60-degree step (inverted polarity)
    # For inverted: high states are flipped from normal
    hall_states = [
        [1, 0, 1, 0, 1, 0],  # Hall 1
        [0, 1, 1, 0, 0, 1],  # Hall 2
        [1, 1, 0, 0, 1, 1],  # Hall 3 (simplified pattern)
    ]

    # Draw angle labels
    lx = fx + 80
    for i, lbl in enumerate(labels):
        draw.text((lx + i * block_w + 12, y), lbl, fill="black", font=fonts["small"])
    y += 16

    # Draw hall sensor rows
    for hi in range(3):
        draw.text((fx + 10, y + 2), hall_labels[hi], fill="black", font=fonts["small"])
        for si in range(6):
            bx = lx + si * block_w
            color = "#CC0000" if hall_states[hi][si] else "white"
            draw.rectangle([bx, y, bx + block_w, y + block_h],
                           fill=color, outline=C_SUNKEN_MID)
        y += block_h + 4

    # Rotor position indicator bar
    y += 4
    draw.text((fx + 10, y + 2), "Rotor", fill="black", font=fonts["small"])
    rotor_colors = ["#CC0000", "#FFFFFF", "#CC0000", "#FFFFFF", "#CC0000", "#FFFFFF"]
    for si in range(6):
        bx = lx + si * block_w
        draw.rectangle([bx, y, bx + block_w, y + block_h],
                       fill=rotor_colors[si], outline=C_SUNKEN_MID)

    img.save("docs/escon_generated/02_rotor_position.png")
    print("Saved 02_rotor_position.png")


def gen_03_speed_sensor():
    """03: Speed Sensor - Available Hall Sensors."""
    fonts = load_fonts()
    img, draw, fx, fy, fw = make_base(fonts, "Speed Sensor")

    y = fy
    draw.text((fx, y), "Speed Sensor", fill="black", font=fonts["heading"])
    y += 30

    draw.text((fx, y), "Please choose a sensor type.", fill="black", font=fonts["label"])
    y += 22
    draw_dropdown(draw, fonts, fx, y, "Available Hall Sensors", w=240)

    img.save("docs/escon_generated/03_speed_sensor.png")
    print("Saved 03_speed_sensor.png")


def gen_04_mode_of_operation():
    """04: Mode of Operation - Speed Controller (Closed Loop)."""
    fonts = load_fonts()
    img, draw, fx, fy, fw = make_base(fonts, "Mode of Operation")

    y = fy
    draw.text((fx, y), "Mode of Operation", fill="black", font=fonts["heading"])
    y += 30

    draw.text((fx, y), "Please choose mode of operation.", fill="black", font=fonts["label"])
    y += 22
    draw_dropdown(draw, fonts, fx, y, "Speed Controller (Closed Loop)", w=300)
    y += 44

    # Closed-loop diagram (simplified block diagram)
    # Draw a simple feedback loop diagram
    bw, bh = 100, 40
    cx = fx + 60
    cy = y + 20

    # Set value input arrow
    draw.line([cx - 40, cy + bh // 2, cx, cy + bh // 2], fill="black", width=2)
    draw.polygon([(cx - 5, cy + bh // 2 - 4), (cx, cy + bh // 2),
                  (cx - 5, cy + bh // 2 + 4)], fill="black")

    # Sum junction (circle)
    sjx = cx + 10
    sjy = cy + bh // 2
    draw.ellipse([sjx - 10, sjy - 10, sjx + 10, sjy + 10], outline="black", fill="white")
    draw.text((sjx - 3, sjy - 7), "+", fill="black", font=fonts["small"])

    # Arrow to controller
    draw.line([sjx + 10, sjy, sjx + 40, sjy], fill="black", width=2)

    # Controller block
    cbx = sjx + 40
    draw.rectangle([cbx, cy, cbx + bw, cy + bh], outline="black", fill="#E8E8E8")
    draw.text((cbx + 12, cy + 12), "Controller", fill="black", font=fonts["label"])

    # Arrow to motor
    draw.line([cbx + bw, sjy, cbx + bw + 30, sjy], fill="black", width=2)

    # Motor block
    mbx = cbx + bw + 30
    draw.rectangle([mbx, cy, mbx + bw, cy + bh], outline="black", fill="#E8E8E8")
    draw.text((mbx + 25, cy + 12), "Motor", fill="black", font=fonts["label"])

    # Output arrow
    draw.line([mbx + bw, sjy, mbx + bw + 40, sjy], fill="black", width=2)
    draw.polygon([(mbx + bw + 35, sjy - 4), (mbx + bw + 40, sjy),
                  (mbx + bw + 35, sjy + 4)], fill="black")

    # Feedback line down and back
    fbx = mbx + bw + 20
    fby = cy + bh + 30
    draw.line([fbx, sjy, fbx, fby], fill="black", width=2)
    draw.line([fbx, fby, sjx, fby], fill="black", width=2)
    draw.line([sjx, fby, sjx, sjy + 10], fill="black", width=2)

    # Feedback label
    draw.text((fx + 140, fby - 16), "Speed Sensor", fill="black", font=fonts["small"])

    # Minus sign at sum junction
    draw.text((sjx - 14, sjy + 4), "\u2013", fill="black", font=fonts["small"])

    img.save("docs/escon_generated/04_mode_of_operation.png")
    print("Saved 04_mode_of_operation.png")


def gen_05_enable():
    """05: Enable - Enable CCW, Digital Input 2, High active."""
    fonts = load_fonts()
    img, draw, fx, fy, fw = make_base(fonts, "Enable", height=580)

    y = fy
    draw.text((fx, y), "Enable", fill="black", font=fonts["heading"])
    y += 30

    draw.text((fx, y), "Please select enable type.", fill="black", font=fonts["label"])
    y += 22
    draw_dropdown(draw, fonts, fx, y, "Enable CCW", w=220)
    y += 36

    # Enable CCW row with two dropdowns
    draw.text((fx + 10, y + 3), "Enable CCW:", fill="black", font=fonts["label"])
    draw_dropdown(draw, fonts, fx + 130, y, "Digital Input 2", w=150)
    draw_dropdown(draw, fonts, fx + 300, y, "High active", w=130)
    y += 40

    # Set Value Orientation diagram
    draw.text((fx, y), "Set Value Orientation:", fill="black", font=fonts["label"])
    y += 20

    # Draw a simple ramp diagram in a sunken frame
    gx, gy = fx + 20, y
    gw, gh = 350, 160
    draw.rectangle([gx, gy, gx + gw, gy + gh], fill="white", outline=C_SUNKEN_MID)
    draw.line([gx, gy, gx + gw, gy], fill=C_SUNKEN_DARK)
    draw.line([gx, gy, gx, gy + gh], fill=C_SUNKEN_DARK)

    # Axes
    ax_l, ax_b = gx + 40, gy + gh - 25
    ax_r, ax_t = gx + gw - 15, gy + 15
    draw.line([ax_l, ax_b, ax_r, ax_b], fill="black", width=1)
    draw.line([ax_l, ax_b, ax_l, ax_t], fill="black", width=1)

    # Axis labels
    draw.text((ax_l - 35, ax_t + 20), "Speed", fill="black", font=fonts["small"])
    draw.text((ax_r - 40, ax_b + 6), "Set Value", fill="black", font=fonts["small"])

    # Draw ramp line (10% to 90% maps to 0 to max speed)
    # The ramp goes from (10%, 0) to (90%, max)
    range_w = ax_r - ax_l
    range_h = ax_b - ax_t - 10

    p10 = ax_l + int(range_w * 0.10)
    p90 = ax_l + int(range_w * 0.90)

    # Red line: CCW direction (rising from 10% to 90%)
    draw.line([p10, ax_b, p90, ax_t + 10], fill="#CC0000", width=2)

    # Labels at 10% and 90%
    draw.text((p10 - 8, ax_b + 4), "10%", fill="black", font=fonts["small"])
    draw.text((p90 - 8, ax_b + 4), "90%", fill="black", font=fonts["small"])

    # Legend
    draw.line([gx + gw - 100, gy + 10, gx + gw - 80, gy + 10], fill="#CC0000", width=2)
    draw.text((gx + gw - 76, gy + 4), "CCW", fill="black", font=fonts["small"])

    img.save("docs/escon_generated/05_enable.png")
    print("Saved 05_enable.png")


def gen_06_set_value():
    """06: Set Value - PWM Set Value, DIN1, 10%=0rpm, 90%=12500rpm."""
    fonts = load_fonts()
    img, draw, fx, fy, fw = make_base(fonts, "Set Value")

    y = fy
    draw.text((fx, y), "Set Value", fill="black", font=fonts["heading"])
    y += 30

    draw.text((fx, y), "Select type of Set Value functionality.", fill="black", font=fonts["label"])
    y += 22
    draw_dropdown(draw, fonts, fx, y, "PWM Set Value", w=240)
    y += 36

    # Input row
    draw.text((fx + 10, y + 3), "Input:", fill="black", font=fonts["label"])
    draw_dropdown(draw, fonts, fx + 130, y, "Digital Input 1", w=150)
    y += 36

    # Speed at 10.0%
    draw.text((fx + 10, y + 3), "Speed at 10.0 %:", fill="black", font=fonts["label"])
    draw_sunken_field(draw, fonts, fx + 250, y, "0.0", w=90)
    draw.text((fx + 348, y + 3), "rpm", fill="black", font=fonts["label"])
    y += 32

    # Speed at 90.0%
    draw.text((fx + 10, y + 3), "Speed at 90.0 %:", fill="black", font=fonts["label"])
    draw_sunken_field(draw, fonts, fx + 250, y, "12500.0", w=90)
    draw.text((fx + 348, y + 3), "rpm", fill="black", font=fonts["label"])
    y += 32

    img.save("docs/escon_generated/06_set_value.png")
    print("Saved 06_set_value.png")


def gen_07_current_limit():
    """07: Current Limit - Fixed Current Limit, 5.0000 A."""
    fonts = load_fonts()
    img, draw, fx, fy, fw = make_base(fonts, "Current Limit")

    y = fy
    draw.text((fx, y), "Current Limit", fill="black", font=fonts["heading"])
    y += 30

    draw.text((fx, y), "Select type of Current Limit functionality:", fill="black",
              font=fonts["label"])
    y += 22
    draw_dropdown(draw, fonts, fx, y, "Fixed Current Limit", w=240)
    y += 36

    # Current Limit field
    draw.text((fx + 10, y + 3), "Current Limit:", fill="black", font=fonts["label"])
    draw_sunken_field(draw, fonts, fx + 250, y, "5.0000", w=90)
    draw.text((fx + 348, y + 3), "A", fill="black", font=fonts["label"])

    img.save("docs/escon_generated/07_current_limit.png")
    print("Saved 07_current_limit.png")


def gen_08_speed_ramp():
    """08: Speed Ramp - Fixed Ramp, Acceleration/Deceleration 5000.0 rpm/s."""
    fonts = load_fonts()
    img, draw, fx, fy, fw = make_base(fonts, "Speed Ramp")

    y = fy
    draw.text((fx, y), "Speed Ramp", fill="black", font=fonts["heading"])
    y += 30

    draw.text((fx, y), "Select type of Ramp functionality.", fill="black", font=fonts["label"])
    y += 22
    draw_dropdown(draw, fonts, fx, y, "Fixed Ramp", w=240)
    y += 36

    # Acceleration field
    draw.text((fx + 10, y + 3), "Acceleration:", fill="black", font=fonts["label"])
    draw_sunken_field(draw, fonts, fx + 250, y, "5000.0", w=90)
    draw.text((fx + 348, y + 3), "rpm/s", fill="black", font=fonts["label"])
    y += 32

    # Deceleration field
    draw.text((fx + 10, y + 3), "Deceleration:", fill="black", font=fonts["label"])
    draw_sunken_field(draw, fonts, fx + 250, y, "5000.0", w=90)
    draw.text((fx + 348, y + 3), "rpm/s", fill="black", font=fonts["label"])

    img.save("docs/escon_generated/08_speed_ramp.png")
    print("Saved 08_speed_ramp.png")


def gen_09_minimal_speed():
    """09: Minimal Speed - 0.0 rpm."""
    fonts = load_fonts()
    img, draw, fx, fy, fw = make_base(fonts, "Minimal Speed")

    y = fy
    draw.text((fx, y), "Minimal Speed", fill="black", font=fonts["heading"])
    y += 30

    draw.text((fx, y), "Due to a small speed sensor resolution the control performance is",
              fill="black", font=fonts["label"])
    y += 18
    draw.text((fx, y), "limited. It may be useful to configure a minimal speed.",
              fill="black", font=fonts["label"])
    y += 30

    # Minimal Speed field
    draw.text((fx + 10, y + 3), "Minimal Speed:", fill="black", font=fonts["label"])
    draw_sunken_field(draw, fonts, fx + 250, y, "0.0", w=90)
    draw.text((fx + 348, y + 3), "rpm", fill="black", font=fonts["label"])

    img.save("docs/escon_generated/09_minimal_speed.png")
    print("Saved 09_minimal_speed.png")


def gen_10_offset():
    """10: Offset - Fixed Offset, 0.0 rpm."""
    fonts = load_fonts()
    img, draw, fx, fy, fw = make_base(fonts, "Offset")

    y = fy
    draw.text((fx, y), "Offset", fill="black", font=fonts["heading"])
    y += 30

    draw.text((fx, y), "Select type of Offset functionality.", fill="black", font=fonts["label"])
    y += 22
    draw_dropdown(draw, fonts, fx, y, "Fixed Offset", w=240)
    y += 36

    # Offset field
    draw.text((fx + 10, y + 3), "Offset:", fill="black", font=fonts["label"])
    draw_sunken_field(draw, fonts, fx + 250, y, "0.0", w=90)
    draw.text((fx + 348, y + 3), "rpm", fill="black", font=fonts["label"])

    img.save("docs/escon_generated/10_offset.png")
    print("Saved 10_offset.png")


def gen_11_digital_io_overview():
    """11: Digital Inputs and Outputs - overview table."""
    fonts = load_fonts()
    img, draw, fx, fy, fw = make_base(
        fonts, "Digital Inputs and Outputs",
        tabs=["Parameters - ESCON", "Data Recorder - ESCON"])

    y = fy
    draw.text((fx, y), "Digital Inputs and Outputs", fill="black", font=fonts["heading"])
    y += 30

    draw.text((fx, y), "Select functionalities for digital inputs and outputs.",
              fill="black", font=fonts["label"])
    y += 22

    headers = ["Input / Output", "Functionality"]
    rows = [
        ["Digital Input 1", "PWM - Set Value"],
        ["Digital Input 2", "Enable CCW"],
        ["Digital Output 3", "Ready"],
        ["Digital Output 4", "Commutation Frequency"],
    ]
    col_widths = [200, 250]
    draw_table(draw, fonts, fx, y, headers, rows, col_widths)

    img.save("docs/escon_generated/11_digital_io_overview.png")
    print("Saved 11_digital_io_overview.png")


def gen_12_digital_output3_ready():
    """12: DOUT3 - Ready, Low active."""
    fonts = load_fonts()
    img, draw, fx, fy, fw = make_base(
        fonts, "Digital Inputs and Outputs",
        tabs=["Parameters - ESCON", "Data Recorder - ESCON"])

    y = fy
    draw.text((fx, y), "Digital Inputs and Outputs", fill="black", font=fonts["heading"])
    y += 30

    draw.text((fx, y), "Select functionalities for digital inputs and outputs.",
              fill="black", font=fonts["label"])
    y += 22

    headers = ["Input / Output", "Functionality"]
    rows = [
        ["Digital Input 1", "PWM - Set Value"],
        ["Digital Input 2", "Enable CCW"],
        ["Digital Output 3", "Ready"],
        ["Digital Output 4", "Commutation Frequency"],
    ]
    col_widths = [200, 250]
    y = draw_table(draw, fonts, fx, y, headers, rows, col_widths, highlight_row=2)
    y += 20

    # Settings for DOUT3
    draw.text((fx, y), "Set the desired settings for the digital output.",
              fill="black", font=fonts["label"])
    y += 24

    draw.text((fx + 10, y + 3), "Polarity:", fill="black", font=fonts["label"])
    draw_dropdown(draw, fonts, fx + 130, y, "Low active", w=150)

    img.save("docs/escon_generated/12_digital_output3_ready.png")
    print("Saved 12_digital_output3_ready.png")


def gen_13_digital_output4_commutation():
    """13: DOUT4 - Commutation Frequency."""
    fonts = load_fonts()
    img, draw, fx, fy, fw = make_base(
        fonts, "Digital Inputs and Outputs",
        tabs=["Parameters - ESCON", "Data Recorder - ESCON"])

    y = fy
    draw.text((fx, y), "Digital Inputs and Outputs", fill="black", font=fonts["heading"])
    y += 30

    draw.text((fx, y), "Select functionalities for digital inputs and outputs.",
              fill="black", font=fonts["label"])
    y += 22

    headers = ["Input / Output", "Functionality"]
    rows = [
        ["Digital Input 1", "PWM - Set Value"],
        ["Digital Input 2", "Enable CCW"],
        ["Digital Output 3", "Ready"],
        ["Digital Output 4", "Commutation Frequency"],
    ]
    col_widths = [200, 250]
    draw_table(draw, fonts, fx, y, headers, rows, col_widths, highlight_row=3)

    img.save("docs/escon_generated/13_digital_output4_commutation.png")
    print("Saved 13_digital_output4_commutation.png")


def gen_14_analog_inputs():
    """14: Analog Inputs - All inputs set to None."""
    fonts = load_fonts()
    img, draw, fx, fy, fw = make_base(fonts, "Analog Inputs")

    y = fy
    draw.text((fx, y), "Analog Inputs", fill="black", font=fonts["heading"])
    y += 30

    draw.text((fx, y), "Select functionalities for analog inputs.",
              fill="black", font=fonts["label"])
    y += 22

    headers = ["Input", "Functionality"]
    rows = [
        ["Analog Input 1", "None"],
        ["Analog Input 2", "None"],
        ["Potentiometer 1", "None"],
        ["Potentiometer 2", "None"],
    ]
    col_widths = [200, 250]
    draw_table(draw, fonts, fx, y, headers, rows, col_widths)

    img.save("docs/escon_generated/14_analog_inputs.png")
    print("Saved 14_analog_inputs.png")


def gen_15_analog_output_current():
    """15: Analog Outputs - Actual Current Averaged, scaling."""
    fonts = load_fonts()
    img, draw, fx, fy, fw = make_base(fonts, "Analog Outputs")

    y = fy
    draw.text((fx, y), "Analog Outputs", fill="black", font=fonts["heading"])
    y += 30

    draw.text((fx, y), "Select functionalities for analog outputs.",
              fill="black", font=fonts["label"])
    y += 22

    headers = ["Output", "Functionality"]
    rows = [
        ["Analog Output 1", "Actual Current Averaged"],
        ["Analog Output 2", "None"],
    ]
    col_widths = [200, 250]
    y = draw_table(draw, fonts, fx, y, headers, rows, col_widths, highlight_row=0)
    y += 20

    # Scaling section
    draw.text((fx, y), "Set scaling for analog output.", fill="black", font=fonts["label"])
    y += 24

    # Current at 0.000 V = 0.0000 A
    draw.text((fx + 10, y + 3), "Current at:", fill="black", font=fonts["label"])
    draw_sunken_field(draw, fonts, fx + 130, y, "0.000", w=70)
    draw.text((fx + 206, y + 3), "V", fill="black", font=fonts["label"])
    draw.text((fx + 228, y + 3), "=", fill="black", font=fonts["label"])
    draw_sunken_field(draw, fonts, fx + 250, y, "0.0000", w=80)
    draw.text((fx + 336, y + 3), "A", fill="black", font=fonts["label"])
    y += 30

    # Current at 3.300 V = 5.2000 A
    draw.text((fx + 10, y + 3), "Current at:", fill="black", font=fonts["label"])
    draw_sunken_field(draw, fonts, fx + 130, y, "3.300", w=70)
    draw.text((fx + 206, y + 3), "V", fill="black", font=fonts["label"])
    draw.text((fx + 228, y + 3), "=", fill="black", font=fonts["label"])
    draw_sunken_field(draw, fonts, fx + 250, y, "5.2000", w=80)
    draw.text((fx + 336, y + 3), "A", fill="black", font=fonts["label"])

    img.save("docs/escon_generated/15_analog_output_current.png")
    print("Saved 15_analog_output_current.png")


if __name__ == "__main__":
    gen_01_motor()
    gen_02_rotor_position()
    gen_03_speed_sensor()
    gen_04_mode_of_operation()
    gen_05_enable()
    gen_06_set_value()
    gen_07_current_limit()
    gen_08_speed_ramp()
    gen_09_minimal_speed()
    gen_10_offset()
    gen_11_digital_io_overview()
    gen_12_digital_output3_ready()
    gen_13_digital_output4_commutation()
    gen_14_analog_inputs()
    gen_15_analog_output_current()
    print("\nAll 15 screenshots generated!")
