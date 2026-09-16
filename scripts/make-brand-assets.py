"""Create Loomik's editable SVG brand assets. Standard-library only.

The README illustration uses demo content, never a real screen/camera capture.
Optional PNG export on macOS: python3 scripts/make-brand-assets.py --png
"""
from pathlib import Path
import argparse
import html
import shutil
import struct
import subprocess

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / 'docs' / 'assets'
ORANGE = '#EF4B2B'
INK = '#26272B'
MUTED = '#747984'


def mark(x=0, y=0, size=128):
    return f'''<g transform="translate({x} {y}) scale({size / 128})">
      <rect width="128" height="128" rx="32" fill="{INK}"/>
      <path d="M37 33V91H91" fill="none" stroke="{ORANGE}" stroke-width="19" stroke-linecap="round" stroke-linejoin="round"/>
      <circle cx="89" cy="37" r="14" fill="{ORANGE}"/>
    </g>'''


def text(x, y, label, size=20, fill=INK, weight=400, **attrs):
    more = ' '.join(f'{key.replace("_", "-")}="{value}"' for key, value in attrs.items())
    return f'<text x="{x}" y="{y}" font-size="{size}" fill="{fill}" font-weight="{weight}" {more}>{html.escape(label)}</text>'


def rect(x, y, width, height, fill, radius=0, **attrs):
    more = ' '.join(f'{key.replace("_", "-")}="{value}"' for key, value in attrs.items())
    return f'<rect x="{x}" y="{y}" width="{width}" height="{height}" rx="{radius}" fill="{fill}" {more}/>'


def document(width, height, body, title, description, defs=''):
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" role="img" aria-labelledby="title description">
<title id="title">{html.escape(title)}</title>
<desc id="description">{html.escape(description)}</desc>
<defs>{defs}</defs>
<g font-family="Arial, Helvetica, sans-serif">{body}</g>
</svg>
'''


def create():
    ASSETS.mkdir(parents=True, exist_ok=True)
    logo = document(512, 512, mark(size=512), 'Loomik logo',
                    'An orange L and recording dot on a rounded graphite square.')
    (ASSETS / 'loomik-logo.svg').write_text(logo)

    parts = [rect(0, 0, 1600, 900, '#F5F6F8', 32)]
    # Brand and editorial copy.
    parts += [mark(82, 74, 76), text(180, 129, 'Loomik', 58, weight=700, letter_spacing='-2.5')]
    parts += [text(82, 284, 'Your screen.', 82, weight=700, letter_spacing='-3.5'),
              text(82, 378, 'Your voice.', 82, weight=700, letter_spacing='-3.5'),
              text(86, 445, 'Record a screen. Add your camera.', 26, MUTED),
              text(86, 483, 'Keep every take on your Mac.', 26, MUTED)]
    parts += [rect(86, 544, 210, 52, INK, 26), text(112, 577, 'Built with Rust', 21, '#FFFFFF', 600),
              text(88, 669, 'MP4  /  MOV  /  MKV', 20, MUTED, 500, letter_spacing='1'),
              text(88, 712, 'No account. No recording time limit.', 21, MUTED)]
    # A demo desktop window; no private screen content is used.
    parts += [rect(705, 177, 812, 476, '#FFFFFF', 24, filter='url(#shadow)'),
              rect(705, 177, 812, 476, 'none', 24, stroke='#E1E4EA'),
              '<path d="M705 237H1517" stroke="#E8EAF0"/>']
    for x, color in [(733, '#F08B80'), (754, '#EFCC79'), (775, '#9ACAA8')]:
        parts.append(f'<circle cx="{x}" cy="207" r="6" fill="{color}"/>')
    parts += [text(816, 213, 'Project walkthrough', 18, MUTED),
              text(750, 300, 'A clear idea,', 37, weight=700, letter_spacing='-1'),
              text(750, 347, 'ready to share.', 37, weight=700, letter_spacing='-1')]
    for y, width in [(389, 300), (414, 336), (439, 251)]:
        parts.append(rect(750, y, width, 8, '#E9EBEF', 4))
    # Small presentation diagram inside the recorded desktop.
    for x, width in [(750, 80), (875, 80), (1000, 80)]:
        parts.append(rect(x, 501, width, 72, '#F6F7F9', 14))
    parts += ['<path d="M776 522H803V542H776Z M782 548H798 M789 542V548" fill="none" stroke="#818896" stroke-width="2.5" stroke-linejoin="round"/>',
              '<path d="M902 522L925 536L902 550Z" fill="#EF4B2B"/>',
              '<path d="M1026 536L1036 546L1055 526" fill="none" stroke="#818896" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"/>',
              '<path d="M841 536H863M966 536H988" stroke="#CDD2DB" stroke-width="2" stroke-dasharray="3 5"/>']
    # Floating settings, using the same visual language as the native egui UI.
    parts += [rect(1165, 111, 306, 441, '#FFFFFF', 24, filter='url(#shadow)'),
              rect(1165, 111, 306, 441, 'none', 24, stroke='#E3E6EC'),
              mark(1187, 133, 30), text(1227, 157, 'Loomik', 21, weight=600),
              '<path d="M1433 143L1444 154M1444 143L1433 154" stroke="#858B95" stroke-width="1.8" stroke-linecap="round"/>',
              rect(1187, 188, 262, 40, '#F1F2F4', 11), rect(1191, 192, 126, 32, '#FFFFFF', 8),
              text(1226, 214, 'Video', 14, '#1966E2'), text(1360, 214, 'Photo', 14, MUTED)]
    for y, label, active in [(248, 'Display 1', False), (310, 'Camera', False), (372, 'Microphone', True)]:
        fill = '#1966E2' if active else '#F1F2F4'
        fg = '#FFFFFF' if active else INK
        parts += [rect(1187, y, 262, 52, fill, 12), text(1240, y + 32, label, 15, fg),
                  f'<path d="M1420 {y+24}L1425 {y+29}L1430 {y+24}" fill="none" stroke="{fg}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>']
        if label == 'Display 1':
            parts += [f'<rect x="1205" y="{y+16}" width="20" height="14" rx="2" fill="none" stroke="{fg}" stroke-width="1.6"/>',
                      f'<path d="M1215 {y+30}V{y+35}M1210 {y+35}H1220" stroke="{fg}" stroke-width="1.6"/>']
        elif label == 'Camera':
            parts += [f'<path d="M1205 {y+18}H1219V{y+32}H1205Z M1219 {y+22}L1225 {y+19}V{y+31}L1219 {y+28}" fill="none" stroke="{fg}" stroke-width="1.6" stroke-linejoin="round"/>']
        else:
            parts += [f'<rect x="1211" y="{y+14}" width="8" height="15" rx="4" fill="none" stroke="{fg}" stroke-width="1.6"/>',
                      f'<path d="M1207 {y+24}V{y+26}A8 8 0 0 0 1223 {y+26}V{y+24}M1215 {y+34}V{y+39}" fill="none" stroke="{fg}" stroke-width="1.6" stroke-linecap="round"/>']
    parts += [text(1191, 447, 'MP4 · 30 fps', 13, MUTED),
              rect(1187, 468, 262, 52, ORANGE, 12), text(1245, 500, 'Start Recording', 16, '#FFFFFF', 600)]
    # Anonymous illustrated webcam circle, clearly a demo rather than a photo.
    parts += ['<circle cx="796" cy="637" r="90" fill="#FFFFFF" filter="url(#shadow)"/>',
              '<g clip-path="url(#camera)">',
              '<circle cx="796" cy="637" r="84" fill="#EFD9C9"/>',
              '<path d="M712 736C716 665 744 652 796 652C847 652 878 665 881 736" fill="#30353F"/>',
              '<path d="M779 638H813V675Q796 690 779 675Z" fill="#C68E72"/>',
              '<ellipse cx="796" cy="613" rx="36" ry="47" fill="#D8A588"/>',
              '<path d="M760 614C747 568 787 554 814 572C838 570 842 602 831 626L822 599C805 606 780 603 770 594Z" fill="#30353F"/>',
              '<path d="M784 627Q796 636 808 627" fill="none" stroke="#915F4C" stroke-width="3" stroke-linecap="round"/>',
              '<circle cx="783" cy="613" r="2.5" fill="#30353F"/><circle cx="808" cy="613" r="2.5" fill="#30353F"/>', '</g>']
    # Floating controls. This overlay is excluded from exported recordings.
    parts += [rect(935, 674, 431, 80, INK, 40, filter='url(#shadow)'),
              '<circle cx="983" cy="714" r="24" fill="#EF4B2B"/>',
              rect(975, 706, 16, 16, '#FFFFFF', 3),
              text(1025, 723, '02:34', 27, '#FFFFFF', 500, font_family='Menlo, Consolas, monospace'),
              '<path d="M1132 701V726M1142 701V726" stroke="#E5E7ED" stroke-width="5" stroke-linecap="round"/>',
              '<path d="M1188 698V730M1197 698V730" stroke="#666A75" stroke-width="1"/>',
              '<path d="M1231 713A12 12 0 1 1 1236 725M1227 702L1232 714L1244 709" fill="none" stroke="#D0D4DF" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>',
              '<path d="M1288 707H1311M1288 720H1311" stroke="#D0D4DF" stroke-width="2" stroke-linecap="round"/>',
              '<circle cx="1295" cy="707" r="3.5" fill="#26272B" stroke="#D0D4DF" stroke-width="1.5"/><circle cx="1305" cy="720" r="3.5" fill="#26272B" stroke="#D0D4DF" stroke-width="1.5"/>']
    parts += ['<path d="M84 806H1516" stroke="#DEE2E8"/>',
              text(86, 851, 'Screen + camera + microphone', 19, MUTED),
              text(1516, 851, 'Native Rust recorder for macOS', 19, MUTED, text_anchor='end')]
    defs = '''<filter id="shadow" x="-25%" y="-25%" width="150%" height="175%" color-interpolation-filters="sRGB"><feDropShadow dx="0" dy="12" stdDeviation="18" flood-color="#283047" flood-opacity=".12"/></filter>
<clipPath id="camera"><circle cx="796" cy="637" r="84"/></clipPath>'''
    banner = document(1600, 900, '\n'.join(parts), 'Loomik — your screen, your voice',
                      'A product illustration showing a demo desktop, floating settings, recording controls and an illustrated webcam circle. No real screen or camera data.', defs)
    (ASSETS / 'loomik-readme.svg').write_text(banner)


def export_png():
    if not shutil.which('qlmanage'):
        raise SystemExit('PNG export requires macOS Quick Look. SVG assets are ready.')
    previews = ROOT / 'target' / 'brand-previews'
    previews.mkdir(parents=True, exist_ok=True)
    for stem, size, expected in [('loomik-logo', 512, (512, 512)), ('loomik-readme', 1600, (1600, 900))]:
        # Quick Look's SVG provider uses a square viewport. Render a square
        # intermediate with vertically centered content, then center-crop it.
        source = (ASSETS / f'{stem}.svg').read_text()
        source = source.replace(f'width="{expected[0]}" height="{expected[1]}" viewBox="0 0 {expected[0]} {expected[1]}"',
                                f'width="{size}" height="{size}" viewBox="0 0 {size} {size}"')
        source = source.replace('<g font-family=', f'<g transform="translate(0 {(size - expected[1]) // 2})" font-family=', 1)
        square = previews / f'{stem}-square.svg'
        square.write_text(source)
        subprocess.run(['qlmanage', '-t', '-s', str(size), '-o', str(previews), str(square)], check=True)
        generated = previews / f'{stem}-square.svg.png'
        destination = ASSETS / f'{stem}.png'
        subprocess.run(['sips', '--cropToHeightWidth', str(expected[1]), str(expected[0]),
                        str(generated), '--out', str(destination)], check=True)
        data = destination.read_bytes()
        dimensions = struct.unpack('>II', data[16:24])
        if dimensions != expected:
            raise SystemExit(f'Unexpected dimensions {dimensions}; expected {expected}.')
        print(f'{stem}.png: {dimensions}, {len(data):,} bytes')



if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--png', action='store_true')
    args = parser.parse_args()
    create()
    if args.png:
        export_png()
