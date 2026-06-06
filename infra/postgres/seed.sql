-- ─── 种子数据（仅开发环境）────────────────────────────────────────────────

-- 创建管理员账号（密码：admin123，bcrypt hash）
INSERT INTO users (id, email, password_hash, name, role, email_verified)
VALUES (
    '00000000-0000-0000-0000-000000000001',
    'admin@novelworld.dev',
    '$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TiGniMnCGkzBMqVbNxoQyJXkBxKi',
    'Admin',
    'admin',
    TRUE
) ON CONFLICT DO NOTHING;
