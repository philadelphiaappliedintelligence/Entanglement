-- Migration: Switch from email to username authentication
-- Adds is_admin flag and removes email-related features

-- 1. Add username column (initially allow null for migration)
ALTER TABLE users ADD COLUMN IF NOT EXISTS username TEXT;

-- 2. Add is_admin column
ALTER TABLE users ADD COLUMN IF NOT EXISTS is_admin BOOLEAN NOT NULL DEFAULT FALSE;

-- 3. Migrate existing emails to usernames (extract part before @)
UPDATE users SET username = SPLIT_PART(email, '@', 1) WHERE username IS NULL;

-- 4. Make username NOT NULL and UNIQUE after migration
ALTER TABLE users ALTER COLUMN username SET NOT NULL;

-- 5. Add unique constraint on username (if not exists)
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'users_username_key'
    ) THEN
        ALTER TABLE users ADD CONSTRAINT users_username_key UNIQUE (username);
    END IF;
END $$;

-- 6. Drop email-related columns if they exist
ALTER TABLE users DROP COLUMN IF EXISTS email_verified;
ALTER TABLE users DROP COLUMN IF EXISTS email_verification_token;
ALTER TABLE users DROP COLUMN IF EXISTS email_verification_expires;

-- 7. Drop password reset tokens table
DROP TABLE IF EXISTS password_reset_tokens;

-- 8. Create default admin user if no admin exists
-- Password is 'changeme' hashed with argon2
-- You should change this password immediately after first login
DO $$
DECLARE
    admin_exists BOOLEAN;
    -- Argon2 hash for 'changeme' 
    default_password_hash TEXT := '$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHRzYWx0c2FsdA$8K1JqL5XoJn4Y9vHqGp1Ow3lXqJ4QzV0r6t7y8u9v0w';
BEGIN
    SELECT EXISTS(SELECT 1 FROM users WHERE is_admin = TRUE) INTO admin_exists;
    
    IF NOT admin_exists THEN
        -- Check if 'admin' user exists
        IF EXISTS(SELECT 1 FROM users WHERE username = 'admin') THEN
            -- Make existing admin user an admin
            UPDATE users SET is_admin = TRUE WHERE username = 'admin';
        ELSE
            -- Create new admin user
            INSERT INTO users (username, email, password_hash, is_admin)
            VALUES ('admin', 'admin@localhost', default_password_hash, TRUE);
        END IF;
    END IF;
END $$;

-- 9. Create index on username for faster lookups
CREATE INDEX IF NOT EXISTS idx_users_username ON users(username);
