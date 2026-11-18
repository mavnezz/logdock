import type { Messages } from './en'

const de: Messages = {
  app: {
    name: 'Logdock',
    tagline: 'Logs, Fehler und Monitoring für deine Laravel-Apps',
  },
  common: {
    loading: 'Wird geladen …',
    retry: 'Erneut versuchen',
    cancel: 'Abbrechen',
    close: 'Schließen',
    save: 'Speichern',
    delete: 'Löschen',
    create: 'Anlegen',
    yes: 'Ja',
    no: 'Nein',
  },
  system: {
    status: 'Status',
    ready: 'Bereit, Daten anzunehmen',
    notReady: 'Nicht bereit',
    problem: {
      database_unavailable: 'Die Datenbank antwortet nicht.',
      low_disk: 'Das Datenverzeichnis ist fast voll.',
      writer_stalled: 'Ein Schreibprozess speichert nichts mehr.',
    },
  },
  problems: {
    network: 'Logdock ist nicht erreichbar.',
    unknown: 'Etwas ist schiefgegangen.',
  },
}

export default de
