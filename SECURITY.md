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
- Passwords hashed with bcrypt (cost factor 12)
- JWT tokens with configurable expiry (default 1 hour)
- Refresh token rotation with server-side storage
- 401 responses do not leak user existence information

### Data Protection
- All SQL queries use parameterized bindings (no string interpolation)
- File upload validates MIME type and enforces size limits (10MB txt, 20MB pdf)
- API gateway enforces rate limiting (configurable, default 500 req/s)

### Infrastructure
- All inter-service communication over internal Docker network
- Only Nginx port (80/443) exposed externally in production
- Database credentials auto-generated on first run
- JWT secret auto-generated (256-bit random)

### LLM Security
- User input passed to LLM prompts includes behavioral constraints
- System prompts instruct models to stay in character and refuse harmful content
- This is defense-in-depth — prompt injection is not fully preventable

### Known Limitations
- Refresh tokens stored in plaintext (not hashed) — acceptable for self-hosted
- No CSRF protection (API-only, no cookie auth)
- No request body size limit on non-upload endpoints
- LLM prompt injection cannot be fully mitigated at the application layer
