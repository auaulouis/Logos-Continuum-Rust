import zipfile
import xml.etree.ElementTree as ET
from pathlib import Path
from docx import Document

DOC = Path('/Users/louis/Logos Rust/tmp/bench-compare/source_docs/Mega LD Backfile Part 1 .docx')

ns = {'w': 'http://schemas.openxmlformats.org/wordprocessingml/2006/main'}

with zipfile.ZipFile(DOC, 'r') as z:
    styles_xml = z.read('word/styles.xml')
    doc_xml = z.read('word/document.xml')

styles_root = ET.fromstring(styles_xml)
style_map = {}
for style in styles_root.findall('.//w:style', ns):
    sid = style.attrib.get('{%s}styleId' % ns['w'], '')
    name_el = style.find('w:name', ns)
    name = name_el.attrib.get('{%s}val' % ns['w'], '') if name_el is not None else ''
    if sid and name:
        style_map[sid] = name

doc_root = ET.fromstring(doc_xml)
paragraphs = []
for p in doc_root.findall('.//w:body/w:p', ns):
    p_style = ''
    pstyle_el = p.find('w:pPr/w:pStyle', ns)
    if pstyle_el is not None:
        p_style = pstyle_el.attrib.get('{%s}val' % ns['w'], '')

    texts = []
    for t in p.findall('.//w:t', ns):
        if t.text:
            texts.append(t.text)
    text = ''.join(texts).rstrip()
    paragraphs.append((p_style, style_map.get(p_style, p_style), text))

print('Rust-style parse preview (first 40):')
for i,(sid,sname,text) in enumerate(paragraphs[:40]):
    print(f'{i:>3} | sid={sid!r:18} style={sname!r:20} | {text[:90]}')

print('\npython-docx preview (first 40):')
d = Document(str(DOC))
for i,p in enumerate(d.paragraphs[:40]):
    print(f'{i:>3} | style={p.style.name!r:20} | {p.text[:90]}')
