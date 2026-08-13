# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| main branch | ✅ |

## Reporting a Vulnerability

**Do NOT open a public issue for security vulnerabilities.**

Instead, please report them responsibly:

1. Email: [security contact via GitHub private vulnerability reporting]
2. Or use GitHub's [private vulnerability reporting](https://github.com/schorsch888/novelworld/security/advisories/new)

Include:
- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)

We will acknowledge receipt within 48 hours and provide a timeline for resolution.

## Security Measures

### Authentication
- Passwords hashed with bcrypt (cost factor 12) in a bounded blocking pool
- JWT tokens with configurable expiry (default 1 hour)
- Refresh tokens are atomically consumed and rotated in server-side storage
- 401 responses do not leak user existence information

### Data Protection
- All SQL queries use parameterized bindings (no string interpolation)
- File upload accepts TXT, EPUB, and PDF with 10 MiB/20 MiB input limits,
  a 20 MiB extracted-text ceiling, bounded blocking parsers, and EPUB aggregate
  expansion and duplicate-spine checks
- Production Nginx enforces 20 requests/second per client with burst 40; the
  Gateway applies its configurable global backstop after authentication on
  protected routes (default 500 requests/second)
- Novel imports, chats, bcrypt, and provider calls have process-wide admission;
  imports also have a fixed provider-call budget
- Retention and application-layer erasure boundaries are documented in
  [docs/DATA_RETENTION.md](./docs/DATA_RETENTION.md)
- Account export uses JWT-derived identity, internal-token-authenticated service
  fragments, explicit field allowlists, a two-request concurrency ceiling, and
  a 15-minute end-to-end deadline. See
  [docs/ACCOUNT_EXPORT.md](./docs/ACCOUNT_EXPORT.md).

### Infrastructure
- All inter-service communication over internal Docker network
- Only Nginx port (80/443) exposed externally in production
- Database credentials auto-generated on first run
- JWT secret auto-generated (256-bit random)

### LLM Security
- User input passed to LLM prompts includes behavioral constraints
- System prompts instruct models to stay in character and refuse harmful content
- Shared provider requests have a 10-second connect timeout, 5-minute total
  deadline, 1 MiB JSON response ceiling, bounded Retry-After, and normalized
  provider errors
- Chat rendering suppresses model-authored Markdown image requests
- This is defense-in-depth — prompt injection is not fully preventable

### Known Limitations
- Refresh tokens stored in plaintext (not hashed) — acceptable for self-hosted
- No CSRF protection (API-only, no cookie auth)
- LLM prompt injection cannot be fully mitigated at the application layer
- Provider logs, provider-hosted generated images, and operator backups are
  outside the application-layer erasure transaction
- Account export snapshots are service-local and sequential, not a globally
  atomic database backup
