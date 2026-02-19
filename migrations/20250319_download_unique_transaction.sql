-- Add unique constraint on transaction_id so INSERT OR IGNORE works correctly
-- (prevents duplicate rows when the same file is downloaded via multiple range requests)
CREATE UNIQUE INDEX IF NOT EXISTS download_transaction_id_unique ON download(transaction_id);
