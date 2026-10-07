# BergPDF privacy policy

*Last changed: 2026-10-07 (version 0.2.2). The Microsoft Store listing links to this page.*

BergPDF is a PDF editor that works on your own computer. The short version: **your documents stay on your computer,
there is no account and there is no telemetry.** The details:

## What BergPDF does not do

* It does not collect, store or send usage statistics, crash reports, device identifiers or advertising identifiers.
* It does not need an account and has no server of its own. The people who make BergPDF receive nothing from it.
* Passwords you type to open or protect a document are used for that and are never saved or sent anywhere.
* It does not upload your documents. Opening, editing, signing, searching, measuring, optimising, converting to
  PDF/A and reading scanned pages with OCR all happen on your computer.

## What is stored on your computer

* **Preferences** (language, theme and other settings) and **recovery copies** of unsaved work, so that a crash does
  not lose it, in BergPDF's folder in your user profile. Recovery copies are deleted when you save or close the document normally.
* **Your AI key**, if you enter one (see below), in its own file in your user profile, with the access rights of that profile. It
  is never written to the preferences, to a link or to a log.
* **OCR models**, if you choose to download them (see below).
* From the Microsoft Store, Windows keeps these files in the app's own storage; uninstalling BergPDF removes them.

## When BergPDF connects to the internet

Only in these cases, and only when you ask for it:

| Feature | When | Where it connects | What is sent |
|---|---|---|---|
| **PDF Copilot** and **Translate** (optional) | Only when you press a button, and only after a one-time consent for the provider you chose | OpenAI, Anthropic, or a server address you typed yourself (for example a model running on your own machine) | The question and the text of the pages needed to answer it, with your own API key. The consent dialog states what is sent. What the provider does with it is governed by *their* privacy policy and your agreement with them. |
| **OCR models** (optional) | When you press *Download* in the OCR dialog | `ocrs-models.s3-accelerate.amazonaws.com` (the model author's public host) | A normal download request. Nothing about you or your documents. The files are checked against a fixed checksum and thrown away if it does not match. |
| **Update check** (not in the Microsoft Store version) | Only if you allowed it, or when you press *Check for Updates* | `api.github.com` | The program name and version number, plus what any web request contains (such as your IP address). |
| **Links** | When you click a link in a document and confirm | The page you opened, in your web browser | Nothing from BergPDF. |

The Microsoft Store version leaves updating to the Microsoft Store and does not contain the update check.

## Your rights and choices

Because BergPDF collects nothing, there is nothing to access, correct or delete on our side. You can delete BergPDF's
local data by uninstalling it, or by removing its folder in your user profile. If you use an AI provider, ask that
provider about the data it holds.

## Children

BergPDF is a general-purpose tool and does not knowingly collect any data from anyone, including children.

## Changes and contact

If this policy changes, the new text is published in this file in the BergPDF repository, with the date above.
Questions: open an issue at <https://github.com/markvdberg0/BergPDF/issues>.
