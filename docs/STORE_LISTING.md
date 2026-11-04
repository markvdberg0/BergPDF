# Microsoft Store listing texts (EN, NL, DE, ZH)

These are the texts of the Store listings in Partner Center (submission for version 0.4). Copy each block into the
matching field of the language's *Store listing*. Limits: description 10,000 characters, short description 1,000,
*What's new* 1,500, each product feature 200 characters, up to 20 features.

The texts only promise what BergPDF does today (see README "Status and limits"). "Free" is only true while the base
price in *Pricing and availability* is 0; change those lines when that changes. The German and Chinese texts have not
been checked by a native speaker. Copyright and trademark info: `© 2026 BergPDF contributors` (all languages).

**Screenshots** (8 per language, made with `tools/store-shots/run.mjs`, see that file), in this order:

| # | File (`<Language>/<Theme>/`) | EN caption | NL caption | DE caption | ZH caption |
|---|---|---|---|---|---|
| 1 | `03-comments-and-stamps-light.png` | Comment, highlight and stamp: dynamic stamps add your name, the date and the time | Opmerkingen, markeringen en stempels: dynamische stempels zetten je naam, de datum en de tijd erop | Kommentieren, markieren und stempeln: dynamische Stempel setzen Ihren Namen, das Datum und die Uhrzeit ein | 批注、高亮和图章：动态图章会自动加上您的姓名、日期和时间 |
| 2 | `02-edit-text-light.png` | Edit the real text of a PDF, right on the page | Bewerk de echte tekst van een pdf, direct op de pagina | Den echten Text einer PDF direkt auf der Seite bearbeiten | 直接在页面上编辑 PDF 中真正的文字 |
| 3 | `04-forms-light.png` | Create and fill in forms | Formulieren maken en invullen | Formulare erstellen und ausfüllen | 创建和填写表单 |
| 4 | `06-compare-light.png` | Compare two versions and see every change | Vergelijk twee versies en zie elke wijziging | Zwei Versionen vergleichen und jede Änderung sehen | 比较两个版本，查看每一处更改 |
| 5 | `07-measure-light.png` | Measure distances and areas on drawings and floor plans | Meet afstanden en oppervlakten op tekeningen en plattegronden | Strecken und Flächen auf Zeichnungen und Grundrissen messen | 在图纸和平面图上测量距离和面积 |
| 6 | `05-headers-and-watermarks-light.png` | Add headers, footers, page numbers and watermarks | Voeg kop- en voetteksten, paginanummers en watermerken toe | Kopf- und Fußzeilen, Seitenzahlen und Wasserzeichen hinzufügen | 添加页眉、页脚、页码和水印 |
| 7 | `01-start-screen-light.png` | Start screen with quick tasks and your recent files | Startscherm met snelle taken en je recente bestanden | Startbildschirm mit schnellen Aufgaben und zuletzt verwendeten Dateien | 开始屏幕：快捷任务和最近文件 |
| 8 | `03-comments-and-stamps-dark.png` | Also in dark mode | Ook in donkere modus | Auch im dunklen Modus | 也支持深色模式 |

## English (en-us)

### Description

A free PDF editor for Windows that edits the real text in your PDFs. No account, no subscription, no cloud: your
documents stay on your own PC.

Fix a typo in a contract, fill in and sign a form, merge two reports, black out personal details or make a scan
searchable. BergPDF does it quickly on your own computer, and every step can be undone.

EDIT
• Change existing text and images right on the page, in any font installed on your PC
• Add text, pictures, headers, footers, page numbers, Bates numbers and watermarks

COMMENT AND REVIEW
• Highlight, underline, strike out, sticky notes, text boxes, callouts, clouds, shapes, arrows and freehand drawing
• Stamps, including dynamic stamps with your name, the date and the time
• Compare two versions of a document and see every inserted, deleted and replaced word

FORMS AND SIGNATURES
• Fill in any PDF form, or create your own: text fields, check boxes, radio buttons, lists, buttons and signature fields, with totals and number and date formats
• Sign with your handwritten signature, or digitally with a .p12/.pfx certificate

PAGES
• Rotate, delete, insert, duplicate, reorder, crop, extract and merge pages
• Edit bookmarks and page labels

SCANS, ARCHIVING AND SECURITY
• OCR makes scanned pages searchable and runs on your own computer
• Save as optimized makes files smaller; convert to PDF/A-2b for long-term archiving
• Redact for real: removed text and images are gone from the file, not just covered up
• Open and create password-protected PDFs (AES-256)

DRAWINGS
• Measure distances, areas, angles and counts on plans, with snapping to corners and lines; export to CSV

SAFE BY DESIGN
• Unlimited undo and redo
• Autosave and crash recovery: unsaved work comes back
• Saving checks the new file before it replaces yours

PRIVATE
No account, no telemetry, no ads. The optional AI features (PDF Copilot to ask questions about a document, and
translation) only work with your own key for OpenAI, Anthropic or a local server, and only send text when you press a
button.

In English, Dutch, German and Simplified Chinese. Open source under the GNU GPL v3; the code is on GitHub.

Good to know: the trust of signature certificates is not checked yet.

### Short description

A free PDF editor that changes the real text in your PDFs. Fill in and sign forms, comment, compare versions,
rearrange pages, make scans searchable and redact for real. Everything happens on your own PC: no account, no
subscription, no cloud.

### What's new in this version

Version 0.4
• A new, calmer interface: a single-line ribbon (the classic ribbon is one switch away) and a start screen with quick tasks and your recent files
• Create forms: text fields, check boxes, radio buttons, lists, buttons and signature fields, with totals and number and date formats
• Form calculations and checks (JavaScript) run only when you allow them, in a closed sandbox that can only fill in the form
• Headers, footers, Bates numbers and watermarks; page labels and cropping
• Edit bookmarks, and compare two versions of a document with the changes marked
• Dynamic stamps with your name, the date and the time
• OCR now reads accented letters (é, ë, ü, ß)

### Product features

* Edit the real text and images in your PDFs, right on the page
* Add text in any font installed on your PC, plus pictures, headers, footers, page numbers and watermarks
* Highlight, underline, strike out, notes, text boxes, callouts, clouds, shapes, arrows and freehand drawing
* Stamps, including dynamic stamps with your name, the date and the time
* Fill in PDF forms, or create your own with text fields, check boxes, lists, buttons and signature fields
* Sign by hand, or digitally with a .p12/.pfx certificate
* Compare two versions of a document and see every change marked
* Rotate, delete, insert, reorder, crop, extract and merge pages; edit bookmarks
* OCR makes scanned pages searchable, on your own computer
* Redact for real: removed text and images are gone from the file
* Make files smaller with Save as optimized; convert to PDF/A-2b for archiving
* Measure distances, areas, angles and counts on drawings, with snapping; export to CSV
* Open and create password-protected PDFs (AES-256)
* Unlimited undo, autosave with crash recovery, and saving that checks the result first
* Free: no account, no subscription, no ads, no telemetry
* In English, Dutch, German and Simplified Chinese; open source under the GPL v3

## Nederlands (nl-nl)

### Beschrijving

Gratis pdf-editor voor Windows die de echte tekst in je pdf's bewerkt. Geen account, geen abonnement, geen cloud: je
documenten blijven op je eigen pc.

Een typfout in een contract verbeteren, een formulier invullen en ondertekenen, twee rapporten samenvoegen,
persoonsgegevens zwartlakken of een scan doorzoekbaar maken. BergPDF doet het snel op je eigen computer, en elke stap
is ongedaan te maken.

BEWERKEN
• Bestaande tekst en afbeeldingen direct op de pagina wijzigen, in elk lettertype dat op je pc staat
• Tekst, afbeeldingen, kop- en voetteksten, paginanummers, Bates-nummers en watermerken toevoegen

OPMERKINGEN EN CONTROLEREN
• Markeren, onderstrepen, doorhalen, notities, tekstvakken, tekstballonnen, wolken, vormen, pijlen en vrij tekenen
• Stempels, ook dynamische stempels met je naam, de datum en de tijd
• Twee versies van een document vergelijken en elk ingevoegd, verwijderd en vervangen woord zien

FORMULIEREN EN HANDTEKENINGEN
• Elk pdf-formulier invullen, of zelf een formulier maken: tekstvelden, selectievakjes, keuzerondjes, lijsten, knoppen en handtekeningvelden, met totalen en getal- en datumnotaties
• Ondertekenen met je handgeschreven handtekening, of digitaal met een .p12/.pfx-certificaat

PAGINA'S
• Pagina's draaien, verwijderen, invoegen, dupliceren, herordenen, bijsnijden, uitpakken en samenvoegen
• Bladwijzers en paginalabels bewerken

SCANS, ARCHIVEREN EN BEVEILIGING
• OCR maakt gescande pagina's doorzoekbaar en draait op je eigen computer
• Opslaan als geoptimaliseerd maakt bestanden kleiner; converteer naar PDF/A-2b voor langdurige archivering
• Echt zwartlakken: verwijderde tekst en afbeeldingen zijn echt weg uit het bestand, niet alleen afgedekt
• Met een wachtwoord beveiligde pdf's openen en maken (AES-256)

TEKENINGEN
• Afstanden, oppervlakten, hoeken en aantallen meten op plattegronden, met vastklikken op hoeken en lijnen; export naar CSV

VEILIG GEBOUWD
• Onbeperkt ongedaan maken en opnieuw
• Automatisch opslaan en herstel na een crash: niet-opgeslagen werk komt terug
• Bij opslaan wordt het nieuwe bestand eerst gecontroleerd, pas daarna vervangt het je bestand

PRIVÉ
Geen account, geen telemetrie, geen advertenties. De optionele AI-functies (PDF Copilot om vragen over een document te
stellen, en vertalen) werken alleen met je eigen sleutel voor OpenAI, Anthropic of een lokale server, en sturen alleen
tekst als je op een knop drukt.

In het Nederlands, Engels, Duits en vereenvoudigd Chinees. Open source onder de GNU GPL v3; de code staat op GitHub.

Goed om te weten: de betrouwbaarheid van handtekeningcertificaten wordt nog niet gecontroleerd.

### Korte beschrijving

Gratis pdf-editor die de echte tekst in je pdf's wijzigt. Vul formulieren in en onderteken ze, plaats opmerkingen,
vergelijk versies, orden pagina's, maak scans doorzoekbaar en lak echt zwart. Alles gebeurt op je eigen pc: geen
account, geen abonnement, geen cloud.

### Wat is er nieuw in deze versie

Versie 0.4
• Een nieuwe, rustigere interface: een lint op één regel (het klassieke lint staat één schakelaar verderop) en een startscherm met snelle taken en je recente bestanden
• Formulieren maken: tekstvelden, selectievakjes, keuzerondjes, lijsten, knoppen en handtekeningvelden, met totalen en getal- en datumnotaties
• Berekeningen en controles in formulieren (JavaScript) draaien alleen als je dat toestaat, in een afgesloten omgeving die alleen het formulier kan invullen
• Kop- en voetteksten, Bates-nummers en watermerken; paginalabels en bijsnijden
• Bladwijzers bewerken, en twee versies van een document vergelijken met de wijzigingen gemarkeerd
• Dynamische stempels met je naam, de datum en de tijd
• OCR leest nu letters met accenten (é, ë, ü, ß)

### Productfuncties

* Bewerk de echte tekst en afbeeldingen in je pdf's, direct op de pagina
* Voeg tekst toe in elk lettertype op je pc, plus afbeeldingen, kop- en voetteksten, paginanummers en watermerken
* Markeren, onderstrepen, doorhalen, notities, tekstvakken, tekstballonnen, wolken, vormen, pijlen en vrij tekenen
* Stempels, ook dynamische stempels met je naam, de datum en de tijd
* Vul pdf-formulieren in, of maak er zelf een met tekstvelden, selectievakjes, lijsten, knoppen en handtekeningvelden
* Onderteken met de hand, of digitaal met een .p12/.pfx-certificaat
* Vergelijk twee versies van een document en zie elke wijziging gemarkeerd
* Pagina's draaien, verwijderen, invoegen, herordenen, bijsnijden, uitpakken en samenvoegen; bladwijzers bewerken
* OCR maakt gescande pagina's doorzoekbaar, op je eigen computer
* Echt zwartlakken: verwijderde tekst en afbeeldingen zijn weg uit het bestand
* Maak bestanden kleiner met Opslaan als geoptimaliseerd; converteer naar PDF/A-2b voor archivering
* Meet afstanden, oppervlakten, hoeken en aantallen op tekeningen, met vastklikken; export naar CSV
* Met een wachtwoord beveiligde pdf's openen en maken (AES-256)
* Onbeperkt ongedaan maken, automatisch opslaan met crashherstel en opslaan dat het resultaat eerst controleert
* Gratis: geen account, geen abonnement, geen advertenties, geen telemetrie
* In het Nederlands, Engels, Duits en vereenvoudigd Chinees; open source onder de GPL v3

## Deutsch (de-de)

### Beschreibung

Kostenloser PDF-Editor für Windows, der den echten Text in Ihren PDFs bearbeitet. Kein Konto, kein Abo, keine Cloud:
Ihre Dokumente bleiben auf Ihrem eigenen PC.

Einen Tippfehler im Vertrag korrigieren, ein Formular ausfüllen und unterschreiben, zwei Berichte zusammenführen,
persönliche Daten schwärzen oder einen Scan durchsuchbar machen: BergPDF erledigt das schnell auf Ihrem eigenen
Computer, und jeder Schritt lässt sich rückgängig machen.

BEARBEITEN
• Vorhandenen Text und Bilder direkt auf der Seite ändern, in jeder auf Ihrem PC installierten Schriftart
• Text, Bilder, Kopf- und Fußzeilen, Seitenzahlen, Bates-Nummern und Wasserzeichen hinzufügen

KOMMENTIEREN UND PRÜFEN
• Markieren, unterstreichen, durchstreichen, Notizen, Textfelder, Legenden, Wolken, Formen, Pfeile und freies Zeichnen
• Stempel, auch dynamische Stempel mit Ihrem Namen, dem Datum und der Uhrzeit
• Zwei Versionen eines Dokuments vergleichen und jedes eingefügte, gelöschte und ersetzte Wort sehen

FORMULARE UND UNTERSCHRIFTEN
• Jedes PDF-Formular ausfüllen oder eigene Formulare erstellen: Textfelder, Kontrollkästchen, Optionsfelder, Listen, Schaltflächen und Signaturfelder, mit Summen sowie Zahlen- und Datumsformaten
• Handschriftlich unterschreiben oder digital mit einem .p12/.pfx-Zertifikat signieren

SEITEN
• Seiten drehen, löschen, einfügen, duplizieren, umsortieren, zuschneiden, extrahieren und zusammenführen
• Lesezeichen und Seitenbeschriftungen bearbeiten

SCANS, ARCHIVIERUNG UND SICHERHEIT
• Die Texterkennung (OCR) macht gescannte Seiten durchsuchbar und läuft auf Ihrem eigenen Computer
• „Optimiert speichern“ verkleinert Dateien; Umwandlung in PDF/A-2b für die Langzeitarchivierung
• Echt schwärzen: entfernter Text und Bilder sind wirklich aus der Datei verschwunden, nicht nur abgedeckt
• Passwortgeschützte PDFs öffnen und erstellen (AES-256)

ZEICHNUNGEN
• Strecken, Flächen, Winkel und Stückzahlen auf Plänen messen, mit Einrasten an Ecken und Linien; Export als CSV

SICHER GEBAUT
• Unbegrenzt rückgängig machen und wiederholen
• Automatisches Speichern und Wiederherstellung nach einem Absturz: ungespeicherte Arbeit kommt zurück
• Beim Speichern wird die neue Datei zuerst geprüft, erst dann ersetzt sie Ihre Datei

PRIVAT
Kein Konto, keine Telemetrie, keine Werbung. Die optionalen KI-Funktionen (PDF Copilot für Fragen zu einem Dokument und
Übersetzen) funktionieren nur mit Ihrem eigenen Schlüssel für OpenAI, Anthropic oder einen lokalen Server und senden
Text nur, wenn Sie auf eine Schaltfläche klicken.

Auf Deutsch, Englisch, Niederländisch und vereinfachtem Chinesisch. Open Source unter der GNU GPL v3; der Code liegt auf
GitHub.

Gut zu wissen: Signaturzertifikate werden noch nicht auf Vertrauenswürdigkeit geprüft.

### Kurzbeschreibung

Kostenloser PDF-Editor, der den echten Text in Ihren PDFs ändert. Formulare ausfüllen und unterschreiben, kommentieren,
Versionen vergleichen, Seiten ordnen, Scans durchsuchbar machen und echt schwärzen. Alles passiert auf Ihrem eigenen
PC: kein Konto, kein Abo, keine Cloud.

### Neuigkeiten in dieser Version

Version 0.4
• Eine neue, ruhigere Oberfläche: ein einzeiliges Menüband (das klassische ist einen Schalter entfernt) und ein Startbildschirm mit schnellen Aufgaben und Ihren zuletzt verwendeten Dateien
• Formulare erstellen: Textfelder, Kontrollkästchen, Optionsfelder, Listen, Schaltflächen und Signaturfelder, mit Summen sowie Zahlen- und Datumsformaten
• Berechnungen und Prüfungen in Formularen (JavaScript) laufen nur, wenn Sie es erlauben, in einer abgeschlossenen Umgebung, die nur das Formular ausfüllen kann
• Kopf- und Fußzeilen, Bates-Nummern und Wasserzeichen; Seitenbeschriftungen und Zuschneiden
• Lesezeichen bearbeiten und zwei Versionen eines Dokuments mit markierten Änderungen vergleichen
• Dynamische Stempel mit Ihrem Namen, dem Datum und der Uhrzeit
• OCR liest jetzt Buchstaben mit Akzenten (é, ë, ü, ß)

### Produktfunktionen

* Den echten Text und die Bilder in Ihren PDFs bearbeiten, direkt auf der Seite
* Text in jeder installierten Schriftart hinzufügen, dazu Bilder, Kopf- und Fußzeilen, Seitenzahlen und Wasserzeichen
* Markieren, unterstreichen, durchstreichen, Notizen, Textfelder, Legenden, Wolken, Formen, Pfeile und freies Zeichnen
* Stempel, auch dynamische Stempel mit Ihrem Namen, dem Datum und der Uhrzeit
* PDF-Formulare ausfüllen oder selbst erstellen, mit Textfeldern, Kontrollkästchen, Listen, Schaltflächen und Signaturfeldern
* Handschriftlich unterschreiben oder digital mit einem .p12/.pfx-Zertifikat signieren
* Zwei Versionen eines Dokuments vergleichen und jede Änderung markiert sehen
* Seiten drehen, löschen, einfügen, umsortieren, zuschneiden, extrahieren und zusammenführen; Lesezeichen bearbeiten
* OCR macht gescannte Seiten durchsuchbar, auf Ihrem eigenen Computer
* Echt schwärzen: entfernter Text und Bilder sind aus der Datei verschwunden
* Dateien mit „Optimiert speichern“ verkleinern; in PDF/A-2b zur Archivierung umwandeln
* Strecken, Flächen, Winkel und Stückzahlen auf Zeichnungen messen, mit Einrasten; Export als CSV
* Passwortgeschützte PDFs öffnen und erstellen (AES-256)
* Unbegrenzt rückgängig machen, automatisches Speichern mit Absturzwiederherstellung und Speichern mit Prüfung
* Kostenlos: kein Konto, kein Abo, keine Werbung, keine Telemetrie
* Auf Deutsch, Englisch, Niederländisch und vereinfachtem Chinesisch; Open Source unter der GPL v3

## 简体中文 (zh-cn)

### 描述

免费的 Windows PDF 编辑器，可以直接编辑 PDF 中真正的文字。无需账户，无需订阅，不用云端：您的文档始终保存在您自己的电脑上。

修改合同里的错别字、填写并签署表单、合并两份报告、涂黑个人信息，或让扫描件可以搜索——BergPDF 都能在您自己的电脑上快速完成，而且每一步都可以撤销。

编辑
• 直接在页面上修改现有的文字和图片，可使用电脑上安装的任何字体
• 添加文字、图片、页眉、页脚、页码、Bates 编号和水印

批注与审阅
• 高亮、下划线、删除线、便笺、文本框、标注框、云线、形状、箭头和自由绘图
• 图章，包括带有您的姓名、日期和时间的动态图章
• 比较文档的两个版本，查看每一处插入、删除和替换的文字

表单与签名
• 填写任何 PDF 表单，或自己创建表单：文本框、复选框、单选按钮、列表、按钮和签名栏，支持合计以及数字和日期格式
• 使用手写签名，或使用 .p12/.pfx 证书进行数字签名

页面
• 旋转、删除、插入、复制、重新排序、裁剪、提取和合并页面
• 编辑书签和页码标签

扫描件、归档与安全
• OCR 文字识别让扫描页面可以搜索，并且在您自己的电脑上运行
• “优化另存为”可减小文件大小；转换为 PDF/A-2b 以便长期归档
• 真正的涂黑：被删除的文字和图片会从文件中彻底移除，而不只是被遮住
• 打开和创建受密码保护的 PDF（AES-256）

图纸
• 在平面图上测量距离、面积、角度和数量，可吸附到角点和线条；可导出为 CSV

安全可靠
• 无限次撤销和重做
• 自动保存与崩溃恢复：未保存的工作可以找回
• 保存时会先检查新文件，确认无误后才替换您的文件

隐私
无需账户，不收集遥测数据，没有广告。可选的 AI 功能（PDF Copilot 用于就文档提问，以及翻译）只能使用您自己的 OpenAI、Anthropic 或本地服务器密钥，并且只有在您点击按钮时才会发送文字。

界面提供简体中文、英语、荷兰语和德语。基于 GNU GPL v3 开源，源代码托管在 GitHub 上。

请注意：目前尚不检查签名证书是否可信。

### 简短描述

免费的 PDF 编辑器，可直接修改 PDF 中真正的文字。填写并签署表单、添加批注、比较版本、整理页面、让扫描件可搜索，并真正涂黑敏感内容。一切都在您自己的电脑上完成：无需账户，无需订阅，不用云端。

### 本版本新功能

0.4 版
• 全新、更简洁的界面：单行功能区（只需一个开关即可切换回经典功能区），以及包含快捷任务和最近文件的开始屏幕
• 创建表单：文本框、复选框、单选按钮、列表、按钮和签名栏，支持合计以及数字和日期格式
• 表单计算和校验（JavaScript）仅在您允许时运行，并且在只能填写表单的封闭沙箱中执行
• 页眉、页脚、Bates 编号和水印；页码标签和裁剪
• 编辑书签，以及比较文档的两个版本并标出更改之处
• 包含您的姓名、日期和时间的动态图章
• OCR 现在可以识别带重音符号的字母（é、ë、ü、ß）

### 产品功能

* 直接在页面上编辑 PDF 中真正的文字和图片
* 使用电脑上安装的任何字体添加文字，还可添加图片、页眉、页脚、页码和水印
* 高亮、下划线、删除线、便笺、文本框、标注框、云线、形状、箭头和自由绘图
* 图章，包括带有您的姓名、日期和时间的动态图章
* 填写 PDF 表单，或用文本框、复选框、列表、按钮和签名栏自己创建表单
* 手写签名，或使用 .p12/.pfx 证书进行数字签名
* 比较文档的两个版本，每一处更改都会被标出
* 旋转、删除、插入、重新排序、裁剪、提取和合并页面；编辑书签
* OCR 文字识别让扫描页面可以搜索，在您自己的电脑上运行
* 真正的涂黑：被删除的文字和图片会从文件中彻底移除
* 用“优化另存为”减小文件；转换为 PDF/A-2b 以便归档
* 在图纸上测量距离、面积、角度和数量，支持吸附；可导出为 CSV
* 打开和创建受密码保护的 PDF（AES-256）
* 无限次撤销、自动保存与崩溃恢复，保存前先检查结果
* 免费：无需账户、无需订阅、没有广告、不收集遥测数据
* 界面提供简体中文、英语、荷兰语和德语；基于 GPL v3 开源
