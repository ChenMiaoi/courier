#!/usr/bin/env python3
"""Generate a small PowerPoint deck from a constrained JSON spec.

This script intentionally avoids third-party dependencies so the repository can
rebuild decks in restricted environments.
"""

from __future__ import annotations

import argparse
import json
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from xml.sax.saxutils import escape
import zipfile

SLIDE_WIDTH = 12_192_000
SLIDE_HEIGHT = 6_858_000
EMU_PER_INCH = 914_400

PALETTE = {
    "ink": "11202A",
    "charcoal": "24313A",
    "cream": "F7F4ED",
    "sand": "EDE6D6",
    "orange": "C65D2E",
    "orange_light": "F3D8CD",
    "teal": "2A7F98",
    "teal_light": "DCEFF4",
    "olive": "617A55",
    "olive_light": "E2E8DB",
    "gold": "D49B32",
    "gold_light": "F5E8C5",
    "slate": "5F6B73",
    "line": "D7CFBE",
    "white": "FFFFFF",
    "danger_light": "F7E1DD",
    "danger": "A74632",
}

DEFAULT_FONT = "Aptos"
TITLE_FONT = "Aptos Display"
CODE_FONT = "Consolas"
EA_FONT = "Microsoft YaHei"

ACCENT_SURFACES = {
    PALETTE["orange"]: PALETTE["orange_light"],
    PALETTE["teal"]: PALETTE["teal_light"],
    PALETTE["olive"]: PALETTE["olive_light"],
    PALETTE["gold"]: PALETTE["gold_light"],
    PALETTE["danger"]: PALETTE["danger_light"],
}


def inches(value: float) -> int:
    return int(value * EMU_PER_INCH)


def color_fragment(color: str) -> str:
    return f'<a:solidFill><a:srgbClr val="{color}"/></a:solidFill>'


def no_fill_fragment() -> str:
    return "<a:noFill/>"


def line_fragment(color: str | None, width: int = 12_700) -> str:
    if color is None:
        return "<a:ln><a:noFill/></a:ln>"
    return (
        f'<a:ln w="{width}" cap="flat" cmpd="sng" algn="ctr">'
        f"{color_fragment(color)}"
        "<a:prstDash val=\"solid\"/>"
        "<a:round/>"
        "</a:ln>"
    )


def text_run(
    text: str,
    *,
    size_pt: float,
    color: str,
    bold: bool = False,
    italic: bool = False,
    font: str = DEFAULT_FONT,
    ea_font: str = EA_FONT,
) -> str:
    bold_attr = ' b="1"' if bold else ""
    italic_attr = ' i="1"' if italic else ""
    size = int(size_pt * 100)
    return (
        f'<a:r><a:rPr lang="zh-CN" altLang="en-US" sz="{size}"'
        f'{bold_attr}{italic_attr} dirty="0" smtClean="0">'
        f"{color_fragment(color)}"
        f'<a:latin typeface="{escape(font)}"/>'
        f'<a:ea typeface="{escape(ea_font)}"/>'
        f'<a:cs typeface="{escape(font)}"/>'
        "</a:rPr>"
        f"<a:t>{escape(text)}</a:t>"
        "</a:r>"
    )


def paragraph_xml(
    text: str,
    *,
    size_pt: float,
    color: str,
    bold: bool = False,
    italic: bool = False,
    font: str = DEFAULT_FONT,
    align: str = "l",
    bullet: bool = False,
    level: int = 0,
) -> str:
    if bullet:
        margin = 342_900 + (level * 228_600)
        indent = -171_450
        ppr = (
            f'<a:pPr marL="{margin}" indent="{indent}" lvl="{level}" algn="{align}">'
            '<a:buChar char="•"/>'
            "</a:pPr>"
        )
    else:
        ppr = f'<a:pPr algn="{align}"><a:buNone/></a:pPr>'
    run = text_run(
        text,
        size_pt=size_pt,
        color=color,
        bold=bold,
        italic=italic,
        font=font,
    )
    end_size = int(size_pt * 100)
    return (
        "<a:p>"
        f"{ppr}"
        f"{run}"
        f'<a:endParaRPr lang="zh-CN" altLang="en-US" sz="{end_size}"/>'
        "</a:p>"
    )


def body_pr(
    *,
    anchor: str = "t",
    left_inset: int = inches(0.12),
    top_inset: int = inches(0.08),
    right_inset: int = inches(0.12),
    bottom_inset: int = inches(0.08),
    wrap: str = "square",
) -> str:
    return (
        f'<a:bodyPr wrap="{wrap}" rtlCol="0" anchor="{anchor}" '
        f'lIns="{left_inset}" tIns="{top_inset}" '
        f'rIns="{right_inset}" bIns="{bottom_inset}">'
        "<a:normAutofit/>"
        "</a:bodyPr>"
    )


@dataclass
class ImageUse:
    rel_id: str
    part_name: str


@dataclass
class SlideDocument:
    elements: list[str] = field(default_factory=list)
    images: list[ImageUse] = field(default_factory=list)
    _shape_id: int = 1

    def next_shape_id(self) -> int:
        self._shape_id += 1
        return self._shape_id

    def add_shape(
        self,
        *,
        x: int,
        y: int,
        cx: int,
        cy: int,
        fill: str | None = None,
        line: str | None = None,
        name: str,
        geometry: str = "rect",
    ) -> None:
        shape_id = self.next_shape_id()
        fill_fragment = no_fill_fragment() if fill is None else color_fragment(fill)
        line_xml = line_fragment(line)
        self.elements.append(
            "<p:sp>"
            "<p:nvSpPr>"
            f'<p:cNvPr id="{shape_id}" name="{escape(name)}"/>'
            "<p:cNvSpPr/>"
            "<p:nvPr/>"
            "</p:nvSpPr>"
            "<p:spPr>"
            f'<a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm>'
            f'<a:prstGeom prst="{geometry}"><a:avLst/></a:prstGeom>'
            f"{fill_fragment}"
            f"{line_xml}"
            "</p:spPr>"
            "</p:sp>"
        )

    def add_text_box(
        self,
        *,
        x: int,
        y: int,
        cx: int,
        cy: int,
        paragraphs: list[dict[str, Any]],
        name: str,
        fill: str | None = None,
        line: str | None = None,
        geometry: str = "rect",
        anchor: str = "t",
        center_text: bool = False,
        font: str = DEFAULT_FONT,
        left_inset: int = inches(0.12),
        top_inset: int = inches(0.08),
        right_inset: int = inches(0.12),
        bottom_inset: int = inches(0.08),
        wrap: str = "square",
    ) -> None:
        shape_id = self.next_shape_id()
        fill_fragment = no_fill_fragment() if fill is None else color_fragment(fill)
        line_xml = line_fragment(line)
        text_xml = []
        for paragraph in paragraphs:
            text_xml.append(
                paragraph_xml(
                    paragraph["text"],
                    size_pt=paragraph["size_pt"],
                    color=paragraph["color"],
                    bold=paragraph.get("bold", False),
                    italic=paragraph.get("italic", False),
                    font=paragraph.get("font", font),
                    align="ctr" if center_text else paragraph.get("align", "l"),
                    bullet=paragraph.get("bullet", False),
                    level=paragraph.get("level", 0),
                )
            )
        self.elements.append(
            "<p:sp>"
            "<p:nvSpPr>"
            f'<p:cNvPr id="{shape_id}" name="{escape(name)}"/>'
            '<p:cNvSpPr txBox="1"/>'
            "<p:nvPr/>"
            "</p:nvSpPr>"
            "<p:spPr>"
            f'<a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm>'
            f'<a:prstGeom prst="{geometry}"><a:avLst/></a:prstGeom>'
            f"{fill_fragment}"
            f"{line_xml}"
            "</p:spPr>"
            "<p:txBody>"
            f"{body_pr(anchor=anchor, left_inset=left_inset, top_inset=top_inset, right_inset=right_inset, bottom_inset=bottom_inset, wrap=wrap)}"
            "<a:lstStyle/>"
            f"{''.join(text_xml)}"
            "</p:txBody>"
            "</p:sp>"
        )

    def add_picture(
        self,
        *,
        x: int,
        y: int,
        cx: int,
        cy: int,
        media_part_name: str,
        name: str,
    ) -> None:
        shape_id = self.next_shape_id()
        rel_id = f"rId{len(self.images) + 2}"
        self.images.append(ImageUse(rel_id=rel_id, part_name=media_part_name))
        self.elements.append(
            "<p:pic>"
            "<p:nvPicPr>"
            f'<p:cNvPr id="{shape_id}" name="{escape(name)}"/>'
            "<p:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></p:cNvPicPr>"
            "<p:nvPr/>"
            "</p:nvPicPr>"
            "<p:blipFill>"
            f'<a:blip r:embed="{rel_id}"/>'
            "<a:stretch><a:fillRect/></a:stretch>"
            "</p:blipFill>"
            "<p:spPr>"
            f'<a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm>'
            '<a:prstGeom prst="rect"><a:avLst/></a:prstGeom>'
            "</p:spPr>"
            "</p:pic>"
        )

    def to_xml(self) -> str:
        return (
            '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            '<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" '
            'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" '
            'xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">'
            "<p:cSld>"
            "<p:spTree>"
            "<p:nvGrpSpPr>"
            '<p:cNvPr id="1" name=""/>'
            "<p:cNvGrpSpPr/>"
            "<p:nvPr/>"
            "</p:nvGrpSpPr>"
            "<p:grpSpPr>"
            "<a:xfrm>"
            '<a:off x="0" y="0"/>'
            '<a:ext cx="0" cy="0"/>'
            '<a:chOff x="0" y="0"/>'
            '<a:chExt cx="0" cy="0"/>'
            "</a:xfrm>"
            "</p:grpSpPr>"
            f"{''.join(self.elements)}"
            "</p:spTree>"
            "</p:cSld>"
            "<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>"
            "</p:sld>"
        )

    def rels_xml(self) -> str:
        relationships = [
            '<Relationship Id="rId1" '
            'Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" '
            'Target="../slideLayouts/slideLayout1.xml"/>'
        ]
        for image in self.images:
            relationships.append(
                f'<Relationship Id="{image.rel_id}" '
                'Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" '
                f'Target="../media/{image.part_name}"/>'
            )
        return (
            '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
            f"{''.join(relationships)}"
            "</Relationships>"
        )


class MediaRegistry:
    def __init__(self, base_dir: Path) -> None:
        self.base_dir = base_dir
        self._entries: dict[Path, str] = {}

    def register(self, path_text: str) -> str:
        path = (self.base_dir / path_text).resolve()
        if path not in self._entries:
            part_name = f"image{len(self._entries) + 1}{path.suffix.lower()}"
            self._entries[path] = part_name
        return self._entries[path]

    def iter_files(self) -> list[tuple[str, bytes]]:
        return [(part_name, path.read_bytes()) for path, part_name in self._entries.items()]

    def extensions(self) -> set[str]:
        return {path.suffix.lower().lstrip(".") for path in self._entries}


def fit_image_in_box(path: Path, box_width: int, box_height: int) -> tuple[int, int]:
    extension = path.suffix.lower()
    raw = path.read_bytes()
    if extension == ".gif" and len(raw) >= 10:
        width = int.from_bytes(raw[6:8], "little")
        height = int.from_bytes(raw[8:10], "little")
    elif extension == ".png" and len(raw) >= 24:
        width = int.from_bytes(raw[16:20], "big")
        height = int.from_bytes(raw[20:24], "big")
    else:
        return box_width, box_height
    if width <= 0 or height <= 0:
        return box_width, box_height
    scale = min(box_width / width, box_height / height)
    return int(width * scale), int(height * scale)


def accent_surface(accent: str) -> str:
    return ACCENT_SURFACES.get(accent, PALETTE["cream"])


def background_decor_fill(fill: str) -> str:
    if fill in {PALETTE["cream"], PALETTE["orange_light"], PALETTE["gold_light"]}:
        return PALETTE["sand"]
    if fill in {PALETTE["sand"], PALETTE["teal_light"], PALETTE["olive_light"]}:
        return PALETTE["cream"]
    return PALETTE["line"]


def add_content_background(
    slide: SlideDocument,
    *,
    fill: str,
    accent: str,
    decor_fill: str | None = None,
) -> None:
    slide.add_shape(
        x=0,
        y=0,
        cx=SLIDE_WIDTH,
        cy=SLIDE_HEIGHT,
        fill=fill,
        line=None,
        name="Background",
    )
    ornament_fill = background_decor_fill(fill) if decor_fill is None else decor_fill
    slide.add_shape(
        x=SLIDE_WIDTH - inches(3.2),
        y=-inches(0.5),
        cx=inches(3.6),
        cy=inches(2.05),
        fill=ornament_fill,
        line=None,
        name="Top Ornament",
        geometry="ellipse",
    )
    slide.add_shape(
        x=-inches(0.9),
        y=SLIDE_HEIGHT - inches(1.45),
        cx=inches(2.55),
        cy=inches(1.8),
        fill=ornament_fill,
        line=None,
        name="Bottom Ornament",
        geometry="ellipse",
    )
    slide.add_shape(
        x=SLIDE_WIDTH - inches(0.5),
        y=inches(0.58),
        cx=inches(0.13),
        cy=SLIDE_HEIGHT - inches(1.28),
        fill=accent,
        line=None,
        name="Accent Rail",
        geometry="roundRect",
    )


def add_footer(slide: SlideDocument, footer: str, page_number: int) -> None:
    slide.add_shape(
        x=inches(0.75),
        y=SLIDE_HEIGHT - inches(0.55),
        cx=inches(10.2),
        cy=inches(0.02),
        fill=PALETTE["line"],
        line=None,
        name="Footer Rule",
    )
    slide.add_text_box(
        x=inches(0.75),
        y=SLIDE_HEIGHT - inches(0.34),
        cx=inches(9.95),
        cy=inches(0.2),
        paragraphs=[
            {"text": footer, "size_pt": 8.8, "color": PALETTE["slate"]},
        ],
        name="Footer",
    )
    slide.add_text_box(
        x=SLIDE_WIDTH - inches(1.45),
        y=SLIDE_HEIGHT - inches(0.47),
        cx=inches(0.7),
        cy=inches(0.3),
        paragraphs=[
            {
                "text": str(page_number),
                "size_pt": 10.5,
                "color": PALETTE["cream"],
                "bold": True,
                "align": "ctr",
            }
        ],
        name="Page Number",
        center_text=True,
        fill=PALETTE["ink"],
        line=None,
        geometry="roundRect",
        anchor="ctr",
    )


def add_title(slide: SlideDocument, title: str, *, accent: str) -> None:
    slide.add_shape(
        x=inches(0.63),
        y=inches(0.38),
        cx=inches(10.9),
        cy=inches(0.72),
        fill=accent_surface(accent),
        line=None,
        name="Title Banner",
        geometry="roundRect",
    )
    slide.add_shape(
        x=inches(0.86),
        y=inches(0.59),
        cx=inches(0.22),
        cy=inches(0.28),
        fill=accent,
        line=None,
        name="Title Accent",
        geometry="roundRect",
    )
    slide.add_text_box(
        x=inches(1.14),
        y=inches(0.48),
        cx=inches(9.95),
        cy=inches(0.45),
        paragraphs=[
            {
                "text": title,
                "size_pt": 23,
                "color": PALETTE["ink"],
                "bold": True,
                "font": TITLE_FONT,
            }
        ],
        name="Slide Title",
        font=TITLE_FONT,
    )


def add_card_text_box(
    slide: SlideDocument,
    *,
    x: int,
    y: int,
    cx: int,
    cy: int,
    paragraphs: list[dict[str, Any]],
    name: str,
    fill: str,
    line: str | None,
    geometry: str = "roundRect",
    anchor: str = "t",
    center_text: bool = False,
    font: str = DEFAULT_FONT,
    shadow_fill: str | None = None,
    shadow_dx: int = inches(0.06),
    shadow_dy: int = inches(0.06),
    left_inset: int = inches(0.16),
    top_inset: int = inches(0.14),
    right_inset: int = inches(0.16),
    bottom_inset: int = inches(0.12),
) -> None:
    if shadow_fill is not None:
        slide.add_shape(
            x=x + shadow_dx,
            y=y + shadow_dy,
            cx=cx,
            cy=cy,
            fill=shadow_fill,
            line=None,
            name=f"{name} Shadow",
            geometry=geometry,
        )
    slide.add_text_box(
        x=x,
        y=y,
        cx=cx,
        cy=cy,
        paragraphs=paragraphs,
        name=name,
        fill=fill,
        line=line,
        geometry=geometry,
        anchor=anchor,
        center_text=center_text,
        font=font,
        left_inset=left_inset,
        top_inset=top_inset,
        right_inset=right_inset,
        bottom_inset=bottom_inset,
    )


def add_bullet_panel(
    slide: SlideDocument,
    *,
    x: int,
    y: int,
    w: int,
    h: int,
    title: str,
    bullets: list[str],
    fill: str,
    title_color: str,
    body_color: str,
    name: str,
    shadow_fill: str,
) -> None:
    paragraphs: list[dict[str, Any]] = [
        {"text": title, "size_pt": 18, "color": title_color, "bold": True, "font": TITLE_FONT}
    ]
    for bullet in bullets:
        paragraphs.append(
            {
                "text": bullet,
                "size_pt": 14,
                "color": body_color,
                "bullet": True,
            }
        )
    add_card_text_box(
        slide,
        x=x,
        y=y,
        cx=w,
        cy=h,
        paragraphs=paragraphs,
        name=name,
        fill=fill,
        line=PALETTE["line"],
        geometry="roundRect",
        shadow_fill=shadow_fill,
    )


def render_title_slide(
    slide: SlideDocument,
    data: dict[str, Any],
    media: MediaRegistry,
    page_number: int,
) -> None:
    slide.add_shape(
        x=0,
        y=0,
        cx=SLIDE_WIDTH,
        cy=SLIDE_HEIGHT,
        fill=PALETTE["ink"],
        line=None,
        name="Background",
    )
    slide.add_shape(
        x=SLIDE_WIDTH - inches(3.35),
        y=-inches(0.48),
        cx=inches(3.8),
        cy=inches(2.25),
        fill=PALETTE["charcoal"],
        line=None,
        name="Top Ornament",
        geometry="ellipse",
    )
    slide.add_shape(
        x=SLIDE_WIDTH - inches(2.08),
        y=inches(0.68),
        cx=inches(1.35),
        cy=inches(1.35),
        fill=PALETTE["orange"],
        line=None,
        name="Accent Orb",
        geometry="ellipse",
    )
    slide.add_shape(
        x=-inches(0.7),
        y=SLIDE_HEIGHT - inches(1.65),
        cx=inches(2.4),
        cy=inches(1.9),
        fill=PALETTE["charcoal"],
        line=None,
        name="Bottom Ornament",
        geometry="ellipse",
    )
    slide.add_shape(
        x=0,
        y=0,
        cx=inches(0.36),
        cy=SLIDE_HEIGHT,
        fill=PALETTE["orange"],
        line=None,
        name="Left Band",
    )
    slide.add_text_box(
        x=inches(0.8),
        y=inches(0.68),
        cx=inches(3.0),
        cy=inches(0.25),
        paragraphs=[
            {
                "text": data["eyebrow"],
                "size_pt": 13.5,
                "color": PALETTE["gold"],
                "bold": True,
                "font": TITLE_FONT,
            }
        ],
        name="Eyebrow",
    )
    slide.add_shape(
        x=inches(0.82),
        y=inches(0.96),
        cx=inches(0.9),
        cy=inches(0.07),
        fill=PALETTE["gold"],
        line=None,
        name="Eyebrow Accent",
        geometry="roundRect",
    )
    slide.add_text_box(
        x=inches(0.8),
        y=inches(1.12),
        cx=inches(6.0),
        cy=inches(1.55),
        paragraphs=[
            {
                "text": line,
                "size_pt": 30,
                "color": PALETTE["cream"],
                "bold": True,
                "font": TITLE_FONT,
            }
            for line in data["title_lines"]
        ],
        name="Hero Title",
        font=TITLE_FONT,
    )
    add_card_text_box(
        slide,
        x=inches(0.82),
        y=inches(2.7),
        cx=inches(5.72),
        cy=inches(0.95),
        paragraphs=[
            {"text": data["subtitle"], "size_pt": 16, "color": PALETTE["cream"]},
            {"text": data["tagline"], "size_pt": 13.5, "color": PALETTE["gold_light"]},
        ],
        name="Hero Subtitle",
        fill=PALETTE["charcoal"],
        line=PALETTE["slate"],
        shadow_fill=PALETTE["ink"],
        top_inset=inches(0.16),
        bottom_inset=inches(0.12),
    )
    chip_x = inches(0.82)
    chip_y = inches(3.88)
    chip_widths = [inches(1.55), inches(1.45), inches(1.4)]
    chip_colors = [PALETTE["teal"], PALETTE["olive"], PALETTE["orange"]]
    for label, chip_width, chip_color in zip(data["chips"], chip_widths, chip_colors):
        slide.add_text_box(
            x=chip_x,
            y=chip_y,
            cx=chip_width,
            cy=inches(0.42),
            paragraphs=[
                {
                    "text": label,
                    "size_pt": 11.5,
                    "color": PALETTE["white"],
                    "bold": True,
                    "align": "ctr",
                }
            ],
            name=f"Chip {label}",
            fill=chip_color,
            line=None,
            geometry="roundRect",
            anchor="ctr",
            center_text=True,
        )
        chip_x += chip_width + inches(0.14)
    image_frame_x = inches(7.25)
    image_frame_y = inches(0.95)
    image_frame_w = inches(5.05)
    image_frame_h = inches(4.55)
    slide.add_shape(
        x=image_frame_x - inches(0.18),
        y=image_frame_y - inches(0.02),
        cx=image_frame_w + inches(0.28),
        cy=image_frame_h + inches(0.28),
        fill=PALETTE["orange"],
        line=None,
        name="Image Shadow",
        geometry="roundRect",
    )
    slide.add_shape(
        x=image_frame_x - inches(0.1),
        y=image_frame_y - inches(0.1),
        cx=image_frame_w + inches(0.2),
        cy=image_frame_h + inches(0.2),
        fill=PALETTE["cream"],
        line=None,
        name="Image Frame",
        geometry="roundRect",
    )
    image_path = (media.base_dir / data["image"]).resolve()
    media_part = media.register(data["image"])
    fitted_w, fitted_h = fit_image_in_box(image_path, image_frame_w, image_frame_h)
    image_x = image_frame_x + (image_frame_w - fitted_w) // 2
    image_y = image_frame_y + (image_frame_h - fitted_h) // 2
    slide.add_picture(
        x=image_x,
        y=image_y,
        cx=fitted_w,
        cy=fitted_h,
        media_part_name=media_part,
        name="CRIEW Demo",
    )
    add_card_text_box(
        slide,
        x=inches(7.28),
        y=inches(5.58),
        cx=inches(4.95),
        cy=inches(0.46),
        paragraphs=[
            {
                "text": data["image_caption"],
                "size_pt": 10.5,
                "color": PALETTE["cream"],
                "italic": True,
            }
        ],
        name="Image Caption",
        fill=PALETTE["charcoal"],
        line=PALETTE["slate"],
        shadow_fill=None,
        top_inset=inches(0.1),
        bottom_inset=inches(0.08),
    )
    add_card_text_box(
        slide,
        x=inches(0.82),
        y=inches(4.96),
        cx=inches(5.88),
        cy=inches(0.78),
        paragraphs=[
            {
                "text": data["closing"],
                "size_pt": 13.5,
                "color": PALETTE["ink"],
                "bold": True,
            }
        ],
        name="Closing Statement",
        fill=PALETTE["gold_light"],
        line=PALETTE["gold"],
        shadow_fill=PALETTE["charcoal"],
    )
    slide.add_shape(
        x=inches(0.82),
        y=SLIDE_HEIGHT - inches(0.55),
        cx=inches(10.35),
        cy=inches(0.02),
        fill=PALETTE["slate"],
        line=None,
        name="Title Footer Rule",
    )
    slide.add_text_box(
        x=inches(0.82),
        y=SLIDE_HEIGHT - inches(0.34),
        cx=inches(7.0),
        cy=inches(0.2),
        paragraphs=[
            {"text": data["footer"], "size_pt": 9.4, "color": PALETTE["sand"]}
        ],
        name="Title Footer",
    )
    slide.add_text_box(
        x=SLIDE_WIDTH - inches(1.45),
        y=SLIDE_HEIGHT - inches(0.47),
        cx=inches(0.72),
        cy=inches(0.3),
        paragraphs=[
            {
                "text": str(page_number),
                "size_pt": 10.5,
                "color": PALETTE["ink"],
                "bold": True,
                "align": "ctr",
            }
        ],
        name="Title Page Number",
        center_text=True,
        fill=PALETTE["cream"],
        line=None,
        geometry="roundRect",
        anchor="ctr",
    )


def render_statement_cards_slide(
    slide: SlideDocument,
    data: dict[str, Any],
    page_number: int,
) -> None:
    add_content_background(slide, fill=PALETTE["cream"], accent=PALETTE["orange"])
    add_title(slide, data["title"], accent=PALETTE["orange"])
    add_card_text_box(
        slide,
        x=inches(0.75),
        y=inches(1.45),
        cx=inches(4.6),
        cy=inches(3.6),
        paragraphs=[
            {
                "text": line,
                "size_pt": 22 if index == 0 else 18,
                "color": PALETTE["cream"],
                "bold": True,
                "font": TITLE_FONT,
            }
            for index, line in enumerate(data["statement_lines"])
        ],
        name="Statement",
        fill=PALETTE["charcoal"],
        line=PALETTE["slate"],
        geometry="roundRect",
        font=TITLE_FONT,
        shadow_fill=PALETTE["sand"],
        top_inset=inches(0.22),
    )
    add_card_text_box(
        slide,
        x=inches(0.95),
        y=inches(4.65),
        cx=inches(4.12),
        cy=inches(0.52),
        paragraphs=[
            {
                "text": data["statement_note"],
                "size_pt": 11.8,
                "color": PALETTE["ink"],
            }
        ],
        name="Statement Note",
        fill=PALETTE["gold_light"],
        line=PALETTE["gold"],
        shadow_fill=PALETTE["sand"],
        top_inset=inches(0.1),
        bottom_inset=inches(0.08),
    )
    card_positions = [
        (inches(5.7), inches(1.45)),
        (inches(9.2), inches(1.45)),
        (inches(5.7), inches(3.3)),
        (inches(9.2), inches(3.3)),
    ]
    card_fills = [
        PALETTE["orange_light"],
        PALETTE["teal_light"],
        PALETTE["olive_light"],
        PALETTE["gold_light"],
    ]
    for (card, (x, y), fill) in zip(data["cards"], card_positions, card_fills):
        add_card_text_box(
            slide,
            x=x,
            y=y,
            cx=inches(2.75),
            cy=inches(1.55),
            paragraphs=[
                {
                    "text": card["title"],
                    "size_pt": 16,
                    "color": PALETTE["ink"],
                    "bold": True,
                    "font": TITLE_FONT,
                },
                {"text": card["body"], "size_pt": 12.5, "color": PALETTE["charcoal"]},
            ],
            name=f'Card {card["title"]}',
            fill=fill,
            line=PALETTE["line"],
            geometry="roundRect",
            shadow_fill=PALETTE["sand"],
        )
    add_card_text_box(
        slide,
        x=inches(5.68),
        y=inches(5.42),
        cx=inches(6.05),
        cy=inches(0.52),
        paragraphs=[
            {"text": data["bottom_note"], "size_pt": 11.2, "color": PALETTE["slate"]}
        ],
        name="Bottom Note",
        fill=PALETTE["white"],
        line=PALETTE["line"],
        shadow_fill=PALETTE["sand"],
        top_inset=inches(0.1),
        bottom_inset=inches(0.08),
    )
    add_footer(slide, data["footer"], page_number)


def render_timeline_slide(
    slide: SlideDocument,
    data: dict[str, Any],
    page_number: int,
) -> None:
    add_content_background(
        slide,
        fill=PALETTE["sand"],
        accent=PALETTE["teal"],
        decor_fill=PALETTE["cream"],
    )
    add_title(slide, data["title"], accent=PALETTE["teal"])
    top_positions = [inches(0.75), inches(3.1), inches(5.45), inches(7.8)]
    bottom_positions = [inches(1.95), inches(4.3), inches(6.65)]
    fills = [
        PALETTE["orange_light"],
        PALETTE["gold_light"],
        PALETTE["teal_light"],
        PALETTE["olive_light"],
        PALETTE["gold_light"],
        PALETTE["teal_light"],
        PALETTE["orange_light"],
    ]
    for index, step in enumerate(data["steps"]):
        if index < 4:
            x = top_positions[index]
            y = inches(1.55)
        else:
            x = bottom_positions[index - 4]
            y = inches(3.55)
        add_card_text_box(
            slide,
            x=x,
            y=y,
            cx=inches(2.15),
            cy=inches(1.35),
            paragraphs=[
                {
                    "text": step["step"],
                    "size_pt": 14,
                    "color": PALETTE["ink"],
                    "bold": True,
                    "font": TITLE_FONT,
                },
                {"text": step["body"], "size_pt": 11.5, "color": PALETTE["charcoal"]},
            ],
            name=f'Step {step["step"]}',
            fill=fills[index],
            line=PALETTE["line"],
            geometry="roundRect",
            shadow_fill=PALETTE["cream"],
        )
    connectors = [
        (inches(2.92), inches(2.1), inches(0.22), inches(0.14), "chevron"),
        (inches(5.27), inches(2.1), inches(0.22), inches(0.14), "chevron"),
        (inches(7.62), inches(2.1), inches(0.22), inches(0.14), "chevron"),
        (inches(9.86), inches(2.56), inches(0.2), inches(0.84), "downArrow"),
        (inches(4.12), inches(4.1), inches(0.22), inches(0.14), "chevron"),
        (inches(6.47), inches(4.1), inches(0.22), inches(0.14), "chevron"),
    ]
    for index, (x, y, w, h, geometry) in enumerate(connectors, start=1):
        slide.add_shape(
            x=x,
            y=y,
            cx=w,
            cy=h,
            fill=PALETTE["teal"],
            line=None,
            name=f"Connector {index}",
            geometry=geometry,
        )
    add_card_text_box(
        slide,
        x=inches(0.75),
        y=inches(5.45),
        cx=inches(11.4),
        cy=inches(0.55),
        paragraphs=[
            {
                "text": data["note"],
                "size_pt": 12.5,
                "color": PALETTE["ink"],
                "bold": True,
            }
        ],
        name="Timeline Note",
        fill=PALETTE["cream"],
        line=PALETTE["line"],
        geometry="roundRect",
        shadow_fill=PALETTE["white"],
        top_inset=inches(0.1),
        bottom_inset=inches(0.08),
    )
    add_footer(slide, data["footer"], page_number)


def render_two_column_bullets_slide(
    slide: SlideDocument,
    data: dict[str, Any],
    page_number: int,
) -> None:
    add_content_background(slide, fill=PALETTE["cream"], accent=PALETTE["gold"])
    add_title(slide, data["title"], accent=PALETTE["gold"])
    add_bullet_panel(
        slide,
        x=inches(0.75),
        y=inches(1.5),
        w=inches(5.25),
        h=inches(3.8),
        title=data["left"]["title"],
        bullets=data["left"]["bullets"],
        fill=PALETTE["danger_light"],
        title_color=PALETTE["danger"],
        body_color=PALETTE["charcoal"],
        name="Left Panel",
        shadow_fill=PALETTE["sand"],
    )
    add_bullet_panel(
        slide,
        x=inches(6.18),
        y=inches(1.5),
        w=inches(5.25),
        h=inches(3.8),
        title=data["right"]["title"],
        bullets=data["right"]["bullets"],
        fill=PALETTE["teal_light"],
        title_color=PALETTE["teal"],
        body_color=PALETTE["charcoal"],
        name="Right Panel",
        shadow_fill=PALETTE["sand"],
    )
    add_card_text_box(
        slide,
        x=inches(2.3),
        y=inches(5.55),
        cx=inches(8.5),
        cy=inches(0.55),
        paragraphs=[
            {
                "text": data["callout"],
                "size_pt": 13,
                "color": PALETTE["ink"],
                "bold": True,
                "align": "ctr",
            }
        ],
        name="Callout",
        fill=PALETTE["gold_light"],
        line=PALETTE["gold"],
        geometry="roundRect",
        anchor="ctr",
        center_text=True,
        shadow_fill=PALETTE["sand"],
        top_inset=inches(0.1),
        bottom_inset=inches(0.08),
    )
    add_footer(slide, data["footer"], page_number)


def render_workflow_slide(
    slide: SlideDocument,
    data: dict[str, Any],
    page_number: int,
) -> None:
    add_content_background(
        slide,
        fill=PALETTE["sand"],
        accent=PALETTE["olive"],
        decor_fill=PALETTE["cream"],
    )
    add_title(slide, data["title"], accent=PALETTE["olive"])
    step_x = inches(0.78)
    fills = [
        PALETTE["orange_light"],
        PALETTE["gold_light"],
        PALETTE["teal_light"],
        PALETTE["olive_light"],
        PALETTE["orange_light"],
    ]
    for index, step in enumerate(data["steps"]):
        add_card_text_box(
            slide,
            x=step_x,
            y=inches(2.02),
            cx=inches(2.1),
            cy=inches(1.65),
            paragraphs=[
                {
                    "text": step["title"],
                    "size_pt": 16,
                    "color": PALETTE["ink"],
                    "bold": True,
                    "font": TITLE_FONT,
                    "align": "ctr",
                },
                {
                    "text": step["body"],
                    "size_pt": 11.5,
                    "color": PALETTE["charcoal"],
                    "align": "ctr",
                },
            ],
            name=f'Workflow {step["title"]}',
            fill=fills[index],
            line=PALETTE["line"],
            geometry="roundRect",
            anchor="ctr",
            center_text=True,
            shadow_fill=PALETTE["cream"],
        )
        if index < len(data["steps"]) - 1:
            slide.add_shape(
                x=step_x + inches(2.15),
                y=inches(2.74),
                cx=inches(0.28),
                cy=inches(0.18),
                fill=PALETTE["teal"],
                line=None,
                name=f"Workflow Connector {index + 1}",
                geometry="chevron",
            )
        step_x += inches(2.38)
    add_card_text_box(
        slide,
        x=inches(1.35),
        y=inches(4.45),
        cx=inches(9.6),
        cy=inches(0.85),
        paragraphs=[
            {
                "text": data["callout"],
                "size_pt": 14,
                "color": PALETTE["cream"],
                "bold": True,
                "align": "ctr",
            }
        ],
        name="Workflow Callout",
        fill=PALETTE["charcoal"],
        line=PALETTE["slate"],
        geometry="roundRect",
        anchor="ctr",
        center_text=True,
        shadow_fill=PALETTE["cream"],
    )
    add_card_text_box(
        slide,
        x=inches(1.52),
        y=inches(5.38),
        cx=inches(9.35),
        cy=inches(0.42),
        paragraphs=[
            {"text": data["note"], "size_pt": 11.5, "color": PALETTE["slate"], "align": "ctr"}
        ],
        name="Workflow Note",
        fill=PALETTE["white"],
        line=PALETTE["line"],
        center_text=True,
        shadow_fill=PALETTE["cream"],
        top_inset=inches(0.1),
        bottom_inset=inches(0.08),
    )
    add_footer(slide, data["footer"], page_number)


def render_card_grid_slide(
    slide: SlideDocument,
    data: dict[str, Any],
    page_number: int,
) -> None:
    add_content_background(slide, fill=PALETTE["cream"], accent=PALETTE["teal"])
    add_title(slide, data["title"], accent=PALETTE["teal"])
    positions = [
        (inches(0.75), inches(1.55)),
        (inches(4.28), inches(1.55)),
        (inches(7.81), inches(1.55)),
        (inches(0.75), inches(3.68)),
        (inches(4.28), inches(3.68)),
        (inches(7.81), inches(3.68)),
    ]
    fills = [
        PALETTE["orange_light"],
        PALETTE["teal_light"],
        PALETTE["olive_light"],
        PALETTE["gold_light"],
        PALETTE["teal_light"],
        PALETTE["orange_light"],
    ]
    for (card, (x, y), fill) in zip(data["cards"], positions, fills):
        add_card_text_box(
            slide,
            x=x,
            y=y,
            cx=inches(3.2),
            cy=inches(1.65),
            paragraphs=[
                {
                    "text": card["title"],
                    "size_pt": 16,
                    "color": PALETTE["ink"],
                    "bold": True,
                    "font": TITLE_FONT,
                },
                {"text": card["body"], "size_pt": 12.2, "color": PALETTE["charcoal"]},
            ],
            name=f'Grid Card {card["title"]}',
            fill=fill,
            line=PALETTE["line"],
            geometry="roundRect",
            shadow_fill=PALETTE["sand"],
        )
    add_card_text_box(
        slide,
        x=inches(0.82),
        y=inches(5.48),
        cx=inches(10.65),
        cy=inches(0.42),
        paragraphs=[
            {"text": data["note"], "size_pt": 11.5, "color": PALETTE["slate"]}
        ],
        name="Grid Note",
        fill=PALETTE["white"],
        line=PALETTE["line"],
        shadow_fill=PALETTE["sand"],
        top_inset=inches(0.1),
        bottom_inset=inches(0.08),
    )
    add_footer(slide, data["footer"], page_number)


def render_code_steps_slide(
    slide: SlideDocument,
    data: dict[str, Any],
    page_number: int,
) -> None:
    add_content_background(
        slide,
        fill=PALETTE["teal_light"],
        accent=PALETTE["orange"],
        decor_fill=PALETTE["cream"],
    )
    add_title(slide, data["title"], accent=PALETTE["orange"])
    add_card_text_box(
        slide,
        x=inches(0.75),
        y=inches(1.55),
        cx=inches(5.0),
        cy=inches(3.5),
        paragraphs=[
            {
                "text": line,
                "size_pt": 18,
                "color": PALETTE["cream"],
                "font": CODE_FONT,
            }
            for line in data["code"].splitlines()
        ],
        name="Command Block",
        fill=PALETTE["charcoal"],
        line=PALETTE["slate"],
        geometry="roundRect",
        font=CODE_FONT,
        shadow_fill=PALETTE["cream"],
        top_inset=inches(0.18),
    )
    add_card_text_box(
        slide,
        x=inches(0.92),
        y=inches(5.12),
        cx=inches(4.82),
        cy=inches(0.48),
        paragraphs=[
            {"text": data["code_note"], "size_pt": 11.5, "color": PALETTE["charcoal"]}
        ],
        name="Code Note",
        fill=PALETTE["white"],
        line=PALETTE["line"],
        shadow_fill=PALETTE["cream"],
        top_inset=inches(0.1),
        bottom_inset=inches(0.08),
    )
    step_y = inches(1.5)
    for index, step in enumerate(data["steps"], start=1):
        slide.add_text_box(
            x=inches(6.1),
            y=step_y,
            cx=inches(0.52),
            cy=inches(0.52),
            paragraphs=[
                {
                    "text": str(index),
                    "size_pt": 15,
                    "color": PALETTE["white"],
                    "bold": True,
                    "align": "ctr",
                }
            ],
            name=f"Step Number {index}",
            fill=PALETTE["orange"],
            line=None,
            geometry="ellipse",
            anchor="ctr",
            center_text=True,
        )
        add_card_text_box(
            slide,
            x=inches(6.72),
            y=step_y - inches(0.04),
            cx=inches(4.62),
            cy=inches(0.6),
            paragraphs=[
                {
                    "text": step,
                    "size_pt": 13,
                    "color": PALETTE["ink"],
                }
            ],
            name=f"Step Text {index}",
            fill=PALETTE["white"],
            line=PALETTE["line"],
            shadow_fill=PALETTE["cream"],
            top_inset=inches(0.11),
            bottom_inset=inches(0.08),
        )
        step_y += inches(0.72)
    add_card_text_box(
        slide,
        x=inches(6.1),
        y=inches(5.45),
        cx=inches(5.25),
        cy=inches(0.42),
        paragraphs=[
            {
                "text": data["callout"],
                "size_pt": 11.8,
                "color": PALETTE["ink"],
                "bold": True,
            }
        ],
        name="Code Callout",
        fill=PALETTE["gold_light"],
        line=PALETTE["gold"],
        geometry="roundRect",
        shadow_fill=PALETTE["cream"],
        top_inset=inches(0.09),
        bottom_inset=inches(0.08),
    )
    add_footer(slide, data["footer"], page_number)


def render_comparison_slide(
    slide: SlideDocument,
    data: dict[str, Any],
    page_number: int,
) -> None:
    add_content_background(slide, fill=PALETTE["cream"], accent=PALETTE["orange"])
    add_title(slide, data["title"], accent=PALETTE["orange"])
    add_bullet_panel(
        slide,
        x=inches(0.75),
        y=inches(1.55),
        w=inches(5.15),
        h=inches(3.75),
        title=data["left"]["title"],
        bullets=data["left"]["bullets"],
        fill=PALETTE["danger_light"],
        title_color=PALETTE["danger"],
        body_color=PALETTE["charcoal"],
        name="Comparison Left",
        shadow_fill=PALETTE["sand"],
    )
    add_bullet_panel(
        slide,
        x=inches(6.28),
        y=inches(1.55),
        w=inches(5.15),
        h=inches(3.75),
        title=data["right"]["title"],
        bullets=data["right"]["bullets"],
        fill=PALETTE["olive_light"],
        title_color=PALETTE["olive"],
        body_color=PALETTE["charcoal"],
        name="Comparison Right",
        shadow_fill=PALETTE["sand"],
    )
    add_card_text_box(
        slide,
        x=inches(1.35),
        y=inches(5.5),
        cx=inches(9.6),
        cy=inches(0.42),
        paragraphs=[
            {
                "text": data["summary"],
                "size_pt": 12.5,
                "color": PALETTE["ink"],
                "bold": True,
                "align": "ctr",
            }
        ],
        name="Comparison Summary",
        fill=PALETTE["gold_light"],
        line=PALETTE["gold"],
        geometry="roundRect",
        anchor="ctr",
        center_text=True,
        shadow_fill=PALETTE["sand"],
        top_inset=inches(0.09),
        bottom_inset=inches(0.08),
    )
    add_footer(slide, data["footer"], page_number)


def render_keypoints_slide(
    slide: SlideDocument,
    data: dict[str, Any],
    page_number: int,
) -> None:
    add_content_background(
        slide,
        fill=PALETTE["sand"],
        accent=PALETTE["teal"],
        decor_fill=PALETTE["cream"],
    )
    add_title(slide, data["title"], accent=PALETTE["teal"])
    add_card_text_box(
        slide,
        x=inches(0.78),
        y=inches(1.45),
        cx=inches(11.0),
        cy=inches(0.72),
        paragraphs=[
            {
                "text": data["lead"],
                "size_pt": 15,
                "color": PALETTE["cream"],
                "bold": True,
                "align": "ctr",
            }
        ],
        name="Lead",
        fill=PALETTE["charcoal"],
        line=PALETTE["slate"],
        geometry="roundRect",
        anchor="ctr",
        center_text=True,
        shadow_fill=PALETTE["cream"],
    )
    positions = [
        (inches(0.78), inches(2.45)),
        (inches(6.3), inches(2.45)),
        (inches(0.78), inches(4.15)),
        (inches(6.3), inches(4.15)),
    ]
    fills = [
        PALETTE["orange_light"],
        PALETTE["teal_light"],
        PALETTE["gold_light"],
        PALETTE["olive_light"],
    ]
    for (point, (x, y), fill) in zip(data["points"], positions, fills):
        add_card_text_box(
            slide,
            x=x,
            y=y,
            cx=inches(5.0),
            cy=inches(1.2),
            paragraphs=[
                {
                    "text": point,
                    "size_pt": 13.2,
                    "color": PALETTE["ink"],
                    "bold": True,
                }
            ],
            name=f"Key Point {point[:10]}",
            fill=fill,
            line=PALETTE["line"],
            geometry="roundRect",
            anchor="ctr",
            center_text=True,
            shadow_fill=PALETTE["cream"],
        )
    add_card_text_box(
        slide,
        x=inches(1.18),
        y=inches(5.52),
        cx=inches(10.0),
        cy=inches(0.46),
        paragraphs=[
            {
                "text": data["closing"],
                "size_pt": 12.2,
                "color": PALETTE["ink"],
                "align": "ctr",
            }
        ],
        name="Keypoint Closing",
        fill=PALETTE["white"],
        line=PALETTE["line"],
        center_text=True,
        shadow_fill=PALETTE["cream"],
        top_inset=inches(0.1),
        bottom_inset=inches(0.08),
    )
    add_footer(slide, data["footer"], page_number)


def render_references_slide(
    slide: SlideDocument,
    data: dict[str, Any],
    page_number: int,
) -> None:
    slide.add_shape(
        x=0,
        y=0,
        cx=SLIDE_WIDTH,
        cy=SLIDE_HEIGHT,
        fill=PALETTE["charcoal"],
        line=None,
        name="Background",
    )
    slide.add_shape(
        x=SLIDE_WIDTH - inches(3.2),
        y=-inches(0.48),
        cx=inches(3.6),
        cy=inches(2.0),
        fill=PALETTE["ink"],
        line=None,
        name="Top Ornament",
        geometry="ellipse",
    )
    slide.add_shape(
        x=-inches(0.8),
        y=SLIDE_HEIGHT - inches(1.5),
        cx=inches(2.4),
        cy=inches(1.85),
        fill=PALETTE["ink"],
        line=None,
        name="Bottom Ornament",
        geometry="ellipse",
    )
    slide.add_shape(
        x=SLIDE_WIDTH - inches(0.5),
        y=inches(0.58),
        cx=inches(0.13),
        cy=SLIDE_HEIGHT - inches(1.28),
        fill=PALETTE["gold"],
        line=None,
        name="Accent Rail",
        geometry="roundRect",
    )
    slide.add_shape(
        x=inches(0.63),
        y=inches(0.38),
        cx=inches(10.9),
        cy=inches(0.72),
        fill=PALETTE["gold_light"],
        line=None,
        name="Title Banner",
        geometry="roundRect",
    )
    slide.add_shape(
        x=inches(0.86),
        y=inches(0.59),
        cx=inches(0.22),
        cy=inches(0.28),
        fill=PALETTE["gold"],
        line=None,
        name="Title Accent",
        geometry="roundRect",
    )
    slide.add_text_box(
        x=inches(1.14),
        y=inches(0.48),
        cx=inches(9.95),
        cy=inches(0.45),
        paragraphs=[
            {
                "text": data["title"],
                "size_pt": 23,
                "color": PALETTE["ink"],
                "bold": True,
                "font": TITLE_FONT,
            }
        ],
        name="Reference Title",
        font=TITLE_FONT,
    )
    midpoint = (len(data["references"]) + 1) // 2
    left_paragraphs: list[dict[str, Any]] = []
    right_paragraphs: list[dict[str, Any]] = []
    for reference in data["references"][:midpoint]:
        left_paragraphs.append(
            {
                "text": f'{reference["label"]}: {reference["url"]}',
                "size_pt": 11.8,
                "color": PALETTE["ink"],
                "bullet": True,
            }
        )
    for reference in data["references"][midpoint:]:
        right_paragraphs.append(
            {
                "text": f'{reference["label"]}: {reference["url"]}',
                "size_pt": 11.8,
                "color": PALETTE["ink"],
                "bullet": True,
            }
        )
    add_card_text_box(
        slide,
        x=inches(0.82),
        y=inches(1.45),
        cx=inches(5.15),
        cy=inches(3.9),
        paragraphs=left_paragraphs,
        name="References Left",
        fill=PALETTE["cream"],
        line=PALETTE["line"],
        shadow_fill=PALETTE["ink"],
        top_inset=inches(0.16),
        left_inset=inches(0.18),
        right_inset=inches(0.18),
    )
    add_card_text_box(
        slide,
        x=inches(6.18),
        y=inches(1.45),
        cx=inches(5.15),
        cy=inches(3.9),
        paragraphs=right_paragraphs,
        name="References Right",
        fill=PALETTE["white"],
        line=PALETTE["line"],
        shadow_fill=PALETTE["ink"],
        top_inset=inches(0.16),
        left_inset=inches(0.18),
        right_inset=inches(0.18),
    )
    add_card_text_box(
        slide,
        x=inches(0.82),
        y=inches(5.52),
        cx=inches(10.55),
        cy=inches(0.44),
        paragraphs=[
            {
                "text": data["note"],
                "size_pt": 11.4,
                "color": PALETTE["ink"],
            }
        ],
        name="Reference Note",
        fill=PALETTE["gold_light"],
        line=PALETTE["gold"],
        shadow_fill=PALETTE["ink"],
        top_inset=inches(0.09),
        bottom_inset=inches(0.08),
    )
    slide.add_shape(
        x=inches(0.82),
        y=SLIDE_HEIGHT - inches(0.55),
        cx=inches(10.35),
        cy=inches(0.02),
        fill=PALETTE["slate"],
        line=None,
        name="Reference Footer Rule",
    )
    slide.add_text_box(
        x=inches(0.82),
        y=SLIDE_HEIGHT - inches(0.34),
        cx=inches(8.0),
        cy=inches(0.2),
        paragraphs=[
            {"text": data["footer"], "size_pt": 9.4, "color": PALETTE["sand"]}
        ],
        name="Reference Footer",
    )
    slide.add_text_box(
        x=SLIDE_WIDTH - inches(1.45),
        y=SLIDE_HEIGHT - inches(0.47),
        cx=inches(0.72),
        cy=inches(0.3),
        paragraphs=[
            {
                "text": str(page_number),
                "size_pt": 10.5,
                "color": PALETTE["ink"],
                "bold": True,
                "align": "ctr",
            }
        ],
        name="Reference Page Number",
        center_text=True,
        fill=PALETTE["cream"],
        line=None,
        geometry="roundRect",
        anchor="ctr",
    )


LAYOUT_RENDERERS = {
    "title": render_title_slide,
    "statement_cards": render_statement_cards_slide,
    "timeline": render_timeline_slide,
    "two_column_bullets": render_two_column_bullets_slide,
    "workflow": render_workflow_slide,
    "card_grid": render_card_grid_slide,
    "code_steps": render_code_steps_slide,
    "comparison": render_comparison_slide,
    "keypoints": render_keypoints_slide,
    "references": render_references_slide,
}


def content_types_xml(slide_count: int, media_extensions: set[str]) -> str:
    defaults = [
        '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>',
        '<Default Extension="xml" ContentType="application/xml"/>',
    ]
    extension_types = {
        "gif": "image/gif",
        "png": "image/png",
        "jpg": "image/jpeg",
        "jpeg": "image/jpeg",
    }
    for extension in sorted(media_extensions):
        content_type = extension_types.get(extension)
        if content_type is not None:
            defaults.append(
                f'<Default Extension="{extension}" ContentType="{content_type}"/>'
            )
    overrides = [
        '<Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>',
        '<Override PartName="/ppt/presProps.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presProps+xml"/>',
        '<Override PartName="/ppt/viewProps.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.viewProps+xml"/>',
        '<Override PartName="/ppt/tableStyles.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.tableStyles+xml"/>',
        '<Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/>',
        '<Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/>',
        '<Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>',
        '<Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>',
        '<Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/>',
    ]
    for slide_index in range(1, slide_count + 1):
        overrides.append(
            f'<Override PartName="/ppt/slides/slide{slide_index}.xml" '
            'ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>'
        )
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
        f"{''.join(defaults)}"
        f"{''.join(overrides)}"
        "</Types>"
    )


def package_rels_xml() -> str:
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>'
        '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>'
        '<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/>'
        "</Relationships>"
    )


def core_xml(spec: dict[str, Any]) -> str:
    created = datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" '
        'xmlns:dc="http://purl.org/dc/elements/1.1/" '
        'xmlns:dcterms="http://purl.org/dc/terms/" '
        'xmlns:dcmitype="http://purl.org/dc/dcmitype/" '
        'xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">'
        f"<dc:title>{escape(spec['metadata']['title'])}</dc:title>"
        f"<dc:creator>{escape(spec['metadata'].get('author', 'OpenAI Codex'))}</dc:creator>"
        "<cp:lastModifiedBy>OpenAI Codex</cp:lastModifiedBy>"
        f'<dcterms:created xsi:type="dcterms:W3CDTF">{created}</dcterms:created>'
        f'<dcterms:modified xsi:type="dcterms:W3CDTF">{created}</dcterms:modified>'
        "</cp:coreProperties>"
    )


def app_xml(slide_count: int) -> str:
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" '
        'xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes">'
        "<Application>Microsoft Office PowerPoint</Application>"
        "<PresentationFormat>On-screen Show (16:9)</PresentationFormat>"
        f"<Slides>{slide_count}</Slides>"
        "<Notes>0</Notes>"
        "<HiddenSlides>0</HiddenSlides>"
        "<MMClips>1</MMClips>"
        "<ScaleCrop>false</ScaleCrop>"
        "<HeadingPairs>"
        '<vt:vector size="2" baseType="variant">'
        "<vt:variant><vt:lpstr>Theme</vt:lpstr></vt:variant>"
        "<vt:variant><vt:i4>1</vt:i4></vt:variant>"
        "</vt:vector>"
        "</HeadingPairs>"
        "<TitlesOfParts>"
        '<vt:vector size="1" baseType="lpstr">'
        "<vt:lpstr>Custom Theme</vt:lpstr>"
        "</vt:vector>"
        "</TitlesOfParts>"
        "<Company>OpenAI</Company>"
        "<LinksUpToDate>false</LinksUpToDate>"
        "<SharedDoc>false</SharedDoc>"
        "<HyperlinksChanged>false</HyperlinksChanged>"
        "<AppVersion>16.0000</AppVersion>"
        "</Properties>"
    )


def presentation_xml(slide_count: int) -> str:
    slide_ids = []
    for slide_index in range(1, slide_count + 1):
        slide_ids.append(
            f'<p:sldId id="{255 + slide_index}" r:id="rId{slide_index + 1}"/>'
        )
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" '
        'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" '
        'xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">'
        "<p:sldMasterIdLst>"
        '<p:sldMasterId id="2147483648" r:id="rId1"/>'
        "</p:sldMasterIdLst>"
        f"<p:sldIdLst>{''.join(slide_ids)}</p:sldIdLst>"
        f'<p:sldSz cx="{SLIDE_WIDTH}" cy="{SLIDE_HEIGHT}"/>'
        '<p:notesSz cx="6858000" cy="9144000"/>'
        "<p:defaultTextStyle>"
        "<a:defPPr/>"
        '<a:lvl1pPr marL="0" algn="l"><a:defRPr sz="1800" kern="1200"/></a:lvl1pPr>'
        '<a:lvl2pPr marL="457200" algn="l"><a:defRPr sz="1600" kern="1200"/></a:lvl2pPr>'
        '<a:lvl3pPr marL="914400" algn="l"><a:defRPr sz="1400" kern="1200"/></a:lvl3pPr>'
        "</p:defaultTextStyle>"
        "</p:presentation>"
    )


def presentation_rels_xml(slide_count: int) -> str:
    relationships = [
        '<Relationship Id="rId1" '
        'Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" '
        'Target="slideMasters/slideMaster1.xml"/>'
    ]
    for slide_index in range(1, slide_count + 1):
        relationships.append(
            f'<Relationship Id="rId{slide_index + 1}" '
            'Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" '
            f'Target="slides/slide{slide_index}.xml"/>'
        )
    relationships.extend(
        [
            f'<Relationship Id="rId{slide_count + 2}" '
            'Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/presProps" '
            'Target="presProps.xml"/>',
            f'<Relationship Id="rId{slide_count + 3}" '
            'Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/viewProps" '
            'Target="viewProps.xml"/>',
            f'<Relationship Id="rId{slide_count + 4}" '
            'Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/tableStyles" '
            'Target="tableStyles.xml"/>',
        ]
    )
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        f"{''.join(relationships)}"
        "</Relationships>"
    )


def pres_props_xml() -> str:
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<p:presentationPr xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" '
        'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" '
        'xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">'
        '<p:showPr loop="0" useTimings="1"/>'
        "</p:presentationPr>"
    )


def view_props_xml() -> str:
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<p:viewPr xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" '
        'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" '
        'xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">'
        "<p:normalViewPr>"
        '<p:restoredLeft sz="15620"/>'
        '<p:restoredTop sz="94660"/>'
        "</p:normalViewPr>"
        "<p:slideViewPr>"
        "<p:cSldViewPr>"
        '<p:cViewPr varScale="1"><p:scale sx="104" sy="104"/><p:origin x="-123" y="-90"/></p:cViewPr>'
        "<p:guideLst/>"
        "</p:cSldViewPr>"
        "</p:slideViewPr>"
        "<p:notesTextViewPr>"
        '<p:cViewPr varScale="1"><p:scale sx="100" sy="100"/><p:origin x="0" y="0"/></p:cViewPr>'
        "</p:notesTextViewPr>"
        '<p:gridSpacing cx="72008" cy="72008"/>'
        "</p:viewPr>"
    )


def table_styles_xml() -> str:
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<a:tblStyleLst xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" '
        'def="{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}"/>'
    )


def theme_xml() -> str:
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="CRIEW Theme">'
        "<a:themeElements>"
        '<a:clrScheme name="CRIEW">'
        f'<a:dk1><a:srgbClr val="{PALETTE["ink"]}"/></a:dk1>'
        f'<a:lt1><a:srgbClr val="{PALETTE["white"]}"/></a:lt1>'
        f'<a:dk2><a:srgbClr val="{PALETTE["charcoal"]}"/></a:dk2>'
        f'<a:lt2><a:srgbClr val="{PALETTE["cream"]}"/></a:lt2>'
        f'<a:accent1><a:srgbClr val="{PALETTE["orange"]}"/></a:accent1>'
        f'<a:accent2><a:srgbClr val="{PALETTE["teal"]}"/></a:accent2>'
        f'<a:accent3><a:srgbClr val="{PALETTE["olive"]}"/></a:accent3>'
        f'<a:accent4><a:srgbClr val="{PALETTE["gold"]}"/></a:accent4>'
        f'<a:accent5><a:srgbClr val="{PALETTE["slate"]}"/></a:accent5>'
        f'<a:accent6><a:srgbClr val="{PALETTE["line"]}"/></a:accent6>'
        '<a:hlink><a:srgbClr val="0563C1"/></a:hlink>'
        '<a:folHlink><a:srgbClr val="954F72"/></a:folHlink>'
        "</a:clrScheme>"
        '<a:fontScheme name="CRIEW Fonts">'
        f'<a:majorFont><a:latin typeface="{TITLE_FONT}"/><a:ea typeface="{EA_FONT}"/><a:cs typeface="{TITLE_FONT}"/></a:majorFont>'
        f'<a:minorFont><a:latin typeface="{DEFAULT_FONT}"/><a:ea typeface="{EA_FONT}"/><a:cs typeface="{DEFAULT_FONT}"/></a:minorFont>'
        "</a:fontScheme>"
        '<a:fmtScheme name="CRIEW Formats">'
        "<a:fillStyleLst>"
        "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>"
        "<a:gradFill rotWithShape=\"1\"><a:gsLst>"
        '<a:gs pos="0"><a:schemeClr val="phClr"><a:tint val="50000"/><a:satMod val="300000"/></a:schemeClr></a:gs>'
        '<a:gs pos="100000"><a:schemeClr val="phClr"><a:shade val="50000"/><a:satMod val="350000"/></a:schemeClr></a:gs>'
        '</a:gsLst><a:lin ang="5400000" scaled="0"/></a:gradFill>'
        "<a:gradFill rotWithShape=\"1\"><a:gsLst>"
        '<a:gs pos="0"><a:schemeClr val="phClr"><a:tint val="80000"/><a:satMod val="300000"/></a:schemeClr></a:gs>'
        '<a:gs pos="100000"><a:schemeClr val="phClr"><a:shade val="30000"/><a:satMod val="200000"/></a:schemeClr></a:gs>'
        '</a:gsLst><a:path path="circle"><a:fillToRect l="50000" t="50000" r="50000" b="50000"/></a:path></a:gradFill>'
        "</a:fillStyleLst>"
        "<a:lnStyleLst>"
        '<a:ln w="9525" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:prstDash val="solid"/></a:ln>'
        '<a:ln w="25400" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:prstDash val="solid"/></a:ln>'
        '<a:ln w="38100" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:prstDash val="solid"/></a:ln>'
        "</a:lnStyleLst>"
        "<a:effectStyleLst>"
        "<a:effectStyle><a:effectLst/></a:effectStyle>"
        "<a:effectStyle><a:effectLst/></a:effectStyle>"
        "<a:effectStyle><a:effectLst/></a:effectStyle>"
        "</a:effectStyleLst>"
        "<a:bgFillStyleLst>"
        "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>"
        "<a:solidFill><a:schemeClr val=\"phClr\"><a:tint val=\"95000\"/><a:satMod val=\"170000\"/></a:schemeClr></a:solidFill>"
        "<a:gradFill rotWithShape=\"1\"><a:gsLst>"
        '<a:gs pos="0"><a:schemeClr val="phClr"><a:tint val="93000"/><a:satMod val="150000"/></a:schemeClr></a:gs>'
        '<a:gs pos="100000"><a:schemeClr val="phClr"><a:tint val="97000"/><a:satMod val="130000"/></a:schemeClr></a:gs>'
        '</a:gsLst><a:lin ang="5400000" scaled="0"/></a:gradFill>'
        "</a:bgFillStyleLst>"
        "</a:fmtScheme>"
        "</a:themeElements>"
        "<a:objectDefaults/>"
        "<a:extraClrSchemeLst/>"
        "</a:theme>"
    )


def slide_master_xml() -> str:
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<p:sldMaster xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" '
        'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" '
        'xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">'
        "<p:cSld name=\"Master\">"
        "<p:spTree>"
        "<p:nvGrpSpPr>"
        '<p:cNvPr id="1" name=""/>'
        "<p:cNvGrpSpPr/>"
        "<p:nvPr/>"
        "</p:nvGrpSpPr>"
        "<p:grpSpPr>"
        "<a:xfrm>"
        '<a:off x="0" y="0"/>'
        '<a:ext cx="0" cy="0"/>'
        '<a:chOff x="0" y="0"/>'
        '<a:chExt cx="0" cy="0"/>'
        "</a:xfrm>"
        "</p:grpSpPr>"
        "</p:spTree>"
        "</p:cSld>"
        '<p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/>'
        "<p:sldLayoutIdLst>"
        '<p:sldLayoutId id="2147483649" r:id="rId1"/>'
        "</p:sldLayoutIdLst>"
        "<p:txStyles>"
        "<p:titleStyle>"
        '<a:lvl1pPr algn="l"><a:defRPr sz="3200" b="1"/></a:lvl1pPr>'
        "</p:titleStyle>"
        "<p:bodyStyle>"
        '<a:lvl1pPr marL="342900" indent="-171450"><a:buChar char="•"/><a:defRPr sz="1800"/></a:lvl1pPr>'
        '<a:lvl2pPr marL="685800" indent="-171450"><a:buChar char="•"/><a:defRPr sz="1600"/></a:lvl2pPr>'
        "</p:bodyStyle>"
        "<p:otherStyle>"
        '<a:lvl1pPr marL="0"><a:defRPr sz="1800"/></a:lvl1pPr>'
        "</p:otherStyle>"
        "</p:txStyles>"
        "</p:sldMaster>"
    )


def slide_master_rels_xml() -> str:
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>'
        '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/>'
        "</Relationships>"
    )


def slide_layout_xml() -> str:
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<p:sldLayout xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" '
        'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" '
        'xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" type="blank" preserve="1">'
        '<p:cSld name="Blank">'
        "<p:spTree>"
        "<p:nvGrpSpPr>"
        '<p:cNvPr id="1" name=""/>'
        "<p:cNvGrpSpPr/>"
        "<p:nvPr/>"
        "</p:nvGrpSpPr>"
        "<p:grpSpPr>"
        "<a:xfrm>"
        '<a:off x="0" y="0"/>'
        '<a:ext cx="0" cy="0"/>'
        '<a:chOff x="0" y="0"/>'
        '<a:chExt cx="0" cy="0"/>'
        "</a:xfrm>"
        "</p:grpSpPr>"
        "</p:spTree>"
        "</p:cSld>"
        "<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>"
        "</p:sldLayout>"
    )


def slide_layout_rels_xml() -> str:
    return (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/>'
        "</Relationships>"
    )


def validate_spec(spec: dict[str, Any]) -> None:
    if "slides" not in spec or not isinstance(spec["slides"], list) or not spec["slides"]:
        raise ValueError("spec must include a non-empty slides list")
    for slide in spec["slides"]:
        layout = slide.get("layout")
        if layout not in LAYOUT_RENDERERS:
            raise ValueError(f"unsupported layout: {layout}")


def build_deck(spec: dict[str, Any], spec_path: Path, output_path: Path) -> None:
    validate_spec(spec)
    media = MediaRegistry(spec_path.parent)
    slides: list[SlideDocument] = []
    for page_number, slide_spec in enumerate(spec["slides"], start=1):
        slide = SlideDocument()
        renderer = LAYOUT_RENDERERS[slide_spec["layout"]]
        if slide_spec["layout"] == "title":
            renderer(slide, slide_spec, media, page_number)
        else:
            renderer(slide, slide_spec, page_number)
        slides.append(slide)

    output_path.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output_path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        archive.writestr(
            "[Content_Types].xml",
            content_types_xml(len(slides), media.extensions()),
        )
        archive.writestr("_rels/.rels", package_rels_xml())
        archive.writestr("docProps/core.xml", core_xml(spec))
        archive.writestr("docProps/app.xml", app_xml(len(slides)))
        archive.writestr("ppt/presentation.xml", presentation_xml(len(slides)))
        archive.writestr(
            "ppt/_rels/presentation.xml.rels",
            presentation_rels_xml(len(slides)),
        )
        archive.writestr("ppt/presProps.xml", pres_props_xml())
        archive.writestr("ppt/viewProps.xml", view_props_xml())
        archive.writestr("ppt/tableStyles.xml", table_styles_xml())
        archive.writestr("ppt/theme/theme1.xml", theme_xml())
        archive.writestr("ppt/slideMasters/slideMaster1.xml", slide_master_xml())
        archive.writestr(
            "ppt/slideMasters/_rels/slideMaster1.xml.rels",
            slide_master_rels_xml(),
        )
        archive.writestr("ppt/slideLayouts/slideLayout1.xml", slide_layout_xml())
        archive.writestr(
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
            slide_layout_rels_xml(),
        )
        for slide_index, slide in enumerate(slides, start=1):
            archive.writestr(f"ppt/slides/slide{slide_index}.xml", slide.to_xml())
            archive.writestr(
                f"ppt/slides/_rels/slide{slide_index}.xml.rels",
                slide.rels_xml(),
            )
        for media_part_name, media_bytes in media.iter_files():
            archive.writestr(f"ppt/media/{media_part_name}", media_bytes)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Generate a simple PPTX from JSON.")
    parser.add_argument("spec", type=Path, help="Path to the JSON slide spec")
    parser.add_argument("output", type=Path, help="Path to the generated .pptx file")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    spec = json.loads(args.spec.read_text(encoding="utf-8"))
    build_deck(spec, args.spec, args.output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
