# DoveClient

Client di posta elettronica per Windows scritto in Rust (porting e miglioramento di DoveClient.py).

## Funzioni
- Più account IMAP/SMTP, anche PEC. Configurazione automatica: provider italiani noti (Libero, Alice/TIM, Virgilio, Tiscali, Poste, Aruba, PEC…), Gmail, Outlook, Yahoo, iCloud, archivio Mozilla ISPDB e prove sulle porte standard; ogni tentativo viene mostrato e verificato con un vero accesso IMAP e SMTP.
- Password salvate **cifrate** (ChaCha20-Poly1305, chiave casuale in `data\.secret.key`).
- Posta in arrivo, Inviati, Preferiti, PEC, Cestino; selezione multipla (Ctrl/Maiusc), ricerca full-text immediata.
- Anteprima (anche da HTML), allegati da salvare, rispondi/inoltra, nuova email con più destinatari, Cc, allegati e **modelli** con segnaposto `{nome}`.
- Esportazione in `.eml` o `.mbox`, statistiche, tema chiaro/scuro, importazione automatica dei `.eml` messi nella cartella `input`.

## Migliorie rispetto alla versione Python
- **Aggiornamento incrementale**: scarica solo i messaggi nuovi (l'originale riscaricava tutto a ogni aggiornamento); i più recenti per primi, con controllo UIDVALIDITY.
- Ricerca sicura (le parole speciali non provocano errori) e per prefisso; messaggi salvati in binario (non base64).
- Copia dei messaggi inviati nella cartella Inviati locale; aggiornamento automatico ogni 5 minuti (impostabile in `data\config.json`).
- Configurazione automatica realmente funzionante (l'originale leggeva l'ISPDB come JSON, ma è XML); timeout brevi nei tentativi.
- Esportazione mbox corretta (mboxrd), nomi di file e allegati ripuliti, ordinamento corretto.

## Da sapere
- Gli account del vecchio programma si leggono, ma le password vanno reinserite (cifratura diversa).
- Non viene mostrato l'HTML con le immagini: l'anteprima è testuale.
- Le cartelle virtuali di Gmail («Tutti i messaggi», «Importanti») sono saltate per evitare doppioni.
- I dati restano accanto all'eseguibile (`data`, `accounts`, `templates`, `export`, `input`).

## Compilare
Metti `costruisci-doveclient.cmd` in `AmbienteRust` e avvialo: scarica da GitHub e compila in `Progetti\DoveClient.it\dist`.
