import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {execFileSync} from 'node:child_process';

function archive(entries) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cwb-document-fixture-'));
  try {
    const contents = path.join(dir, 'contents');
    fs.mkdirSync(contents);
    for (const [name, text] of Object.entries(entries)) {
      const file = path.join(contents, name);
      fs.mkdirSync(path.dirname(file), {recursive:true});
      fs.writeFileSync(file, text);
    }
    const zip = path.join(dir, 'fixture.zip');
    execFileSync('zip', ['-q', '-r', zip, '.'], {cwd:contents});
    return fs.readFileSync(zip);
  } finally { fs.rmSync(dir, {recursive:true, force:true}); }
}

function pdf(text) {
  const stream = `BT /F1 12 Tf 40 760 Td (${text}) Tj ET`;
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>',
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
    `<< /Length ${stream.length} >>\nstream\n${stream}\nendstream`,
  ];
  let value = '%PDF-1.4\n';
  const offsets = [];
  objects.forEach((object, i) => { offsets.push(value.length); value += `${i+1} 0 obj\n${object}\nendobj\n`; });
  const xref = value.length;
  value += 'xref\n0 6\n0000000000 65535 f \n';
  value += offsets.map(n => `${String(n).padStart(10, '0')} 00000 n \n`).join('');
  value += `trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(value);
}

export function documents() {
  const file = (name, buffer) => ({name, mimeType:'application/octet-stream', buffer});
  return [
    file('brief.pdf', pdf('PDF-CONTEXT-724')),
    file('회의.docx', archive({'word/document.xml':'<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>WORD-CONTEXT-725 회의 &amp; notes</w:t></w:r></w:p></w:body></w:document>'})),
    file('budget.xlsx', archive({
      'xl/workbook.xml':'<workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Budget" r:id="rId1"/></sheets></workbook>',
      'xl/_rels/workbook.xml.rels':'<Relationships><Relationship Id="rId1" Target="worksheets/sheet1.xml"/></Relationships>',
      'xl/worksheets/sheet1.xml':'<worksheet><sheetData><row><c r="A1" t="inlineStr"><is><t>SHEET-CONTEXT-726</t></is></c><c r="B1"><v>42</v></c></row></sheetData></worksheet>',
    })),
    file('pitch.pptx', archive({
      'ppt/presentation.xml':'<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst></p:presentation>',
      'ppt/_rels/presentation.xml.rels':'<Relationships><Relationship Id="rId1" Target="slides/slide1.xml"/></Relationships>',
      'ppt/slides/slide1.xml':'<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:p><a:r><a:t>SLIDE-CONTEXT-727</a:t></a:r></a:p></p:sld>',
    })),
    file('notes.odt', archive({'content.xml':'<document><p>OPEN-CONTEXT-728</p></document>'})),
    file('bundle.zip', archive({'folder/archive-reference-729.txt':'NESTED-CONTENT-NOT-READ'})),
    file('.gitignore', Buffer.from('DOTFILE-CONTEXT-730\nnode_modules/')),
    file('README', Buffer.from('EXTENSIONLESS-CONTEXT-731')),
  ];
}
