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

-- 8. Promote existing 'admin' user if no admin exists
-- SECURITY: Hardcoded default credentials removed.
-- Use `tangled user create --username admin --admin` for first-run admin setup.
DO $$
DECLARE
    admin_exists BOOLEAN;
BEGIN
    SELECT EXISTS(SELECT 1 FROM users WHERE is_admin = TRUE) INTO admin_exists;

    IF NOT admin_exists THEN
        -- If an 'admin' user already exists, promote them
        IF EXISTS(SELECT 1 FROM users WHERE username = 'admin') THEN
            UPDATE users SET is_admin = TRUE WHERE username = 'admin';
        END IF;
        -- Otherwise, admin must be created via CLI: tangled user create --username admin --admin
    END IF;
END $$;

-- 9. Create index on username for faster lookups
CREATE INDEX IF NOT EXISTS idx_users_username ON users(username);
