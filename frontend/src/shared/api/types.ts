// Names for the API's shapes, from the generated schema.
import type { components } from './schema'

type Schemas = components['schemas']

export type Account = Schemas['Account']
export type AccountRole = Schemas['AccountRole']
export type Session = Schemas['Session']
export type Project = Schemas['Project']
export type Environment = Schemas['Environment']
export type ProjectSettings = Schemas['Settings']
export type IngestKey = Schemas['Key']
export type CreatedKey = Schemas['CreatedKey']
export type IngestCount = Schemas['IngestCount']
export type AuditEntry = Schemas['AuditEntry']
