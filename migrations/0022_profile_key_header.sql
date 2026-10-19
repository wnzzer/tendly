-- The header an endpoint reads its key from, as JSON {"name","value"} with
-- `{key}` in the value. Absent keeps the protocol's own header.
ALTER TABLE endpoint_profiles ADD COLUMN key_header TEXT;
