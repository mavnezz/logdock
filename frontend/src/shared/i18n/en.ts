// The English texts define the structure every other language must match.
const en = {
  app: {
    name: 'Logdock',
    tagline: 'Logs, errors and monitoring for your Laravel apps',
  },
  common: {
    loading: 'Loading…',
    retry: 'Try again',
    cancel: 'Cancel',
    close: 'Close',
    save: 'Save',
    delete: 'Delete',
    create: 'Create',
    yes: 'Yes',
    no: 'No',
  },
  system: {
    status: 'Status',
    ready: 'Ready to take data',
    notReady: 'Not ready',
    problem: {
      database_unavailable: 'The database does not answer.',
      low_disk: 'The data directory is almost full.',
      writer_stalled: 'A writer has stopped committing.',
    },
  },
  problems: {
    network: 'Logdock cannot be reached.',
    unknown: 'Something went wrong.',
  },
}

export type Messages = typeof en
export default en
