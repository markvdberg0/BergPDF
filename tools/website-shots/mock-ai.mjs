// Stand-in AI service for the website screenshots: translates the report's paragraphs into Dutch.
import http from 'node:http';
const RAW = [
  ['Quarterly Report', 'Kwartaalrapport'],
  ['The quick brown fox jumps over the lazy dog. Pack my box with five dozen liquor jugs. Office efficient workflow: fi fl ffi ligatures are common in typeset documents.',
   'De snelle bruine vos springt over de luie hond. Pak mijn doos met vijf dozijn kruiken likeur. Efficiënte kantoorworkflow: fi-, fl- en ffi-ligaturen komen veel voor in gezette documenten.'],
  ['Second paragraph on page one, with a wider line of text that continues past the end of the line in order to demonstrate wrapping across several lines of body copy produced by a real, independent PDF producer.',
   'Tweede alinea op pagina één, met een bredere tekstregel die doorloopt voorbij het einde van de regel om te laten zien hoe lopende tekst over meerdere regels wordt afgebroken, gemaakt door een echte, onafhankelijke pdf-producent.'],
  ['Revenue Overview', 'Omzetoverzicht'],
  ['Revenue grew by twelve percent over the previous quarter. Unicode check: café naïve résumé Zürich €100.',
   'De omzet groeide met twaalf procent ten opzichte van het vorige kwartaal. Unicode-controle: café naïve résumé Zürich €100.'],
  ['Region', 'Regio'], ['Units', 'Eenheden'], ['Total', 'Totaal'], ['North', 'Noord'], ['South', 'Zuid'],
  ['Appendix', 'Bijlage'],
  ['Confidential appendix: the secret code is ALPHA-7731.', 'Vertrouwelijke bijlage: de geheime code is ALPHA-7731.'],
  ['This sentence exists so that editing and redaction tests have known content.', 'Deze zin bestaat zodat tests voor bewerken en doorhalen bekende inhoud hebben.'],
];
const norm = x => x.normalize("NFKC").trim();
const NL = new Map(RAW.map(([k,v])=>[norm(k),v]));
const port = Number(process.argv[2] || 8099);
http.createServer((req, res) => {
  let body = '';
  req.on('data', c => body += c);
  req.on('end', () => {
    let v = {}; try { v = JSON.parse(body); } catch {}
    const anthropic = v.system !== undefined;
    const user = anthropic ? (v.messages?.at(-1)?.content ?? '') : (v.messages?.at(-1)?.content ?? '');
    const sys = anthropic ? (typeof v.system === 'string' ? v.system : (v.system?.[0]?.text ?? '')) : (v.messages?.[0]?.content ?? '');
    let text = 'OK';
    if (sys.includes('JSON array of text blocks')) {
      let blocks = []; try { blocks = JSON.parse(user); } catch {}
      text = JSON.stringify(blocks.map(b => { const t = NL.get(norm(b)); if (!t) console.error('NO TRANSLATION FOR:', JSON.stringify(b)); return t ?? b; }));
    }
    const resp = anthropic
      ? { content: [{ type: 'text', text }], stop_reason: 'end_turn', usage: { input_tokens: 1, output_tokens: 1 } }
      : { choices: [{ message: { content: text }, finish_reason: 'stop' }], usage: { prompt_tokens: 1, completion_tokens: 1 } };
    setTimeout(() => { res.writeHead(200, { 'content-type': 'application/json' }); res.end(JSON.stringify(resp)); }, 500);
  });
}).listen(port, '127.0.0.1', () => console.error('mock on', port));
