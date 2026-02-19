-- Deduplicate download rows before enforcing uniqueness on transaction_id.
-- In production the UNIQUE INDEX migration may have failed because existing rows
-- had duplicate transaction_ids. This migration cleans them up first.

-- Keep only the row with the highest rowid (most recently inserted) per transaction_id.
DELETE FROM download
WHERE rowid NOT IN (
    SELECT MAX(rowid)
    FROM download
    GROUP BY transaction_id
);

-- Re-create the index in case it was never created (failed migration on first attempt).
CREATE UNIQUE INDEX IF NOT EXISTS download_transaction_id_unique ON download(transaction_id);
