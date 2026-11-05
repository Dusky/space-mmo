-- The world is saved as one JSON document per system and per character.
-- The server owns the format; nothing else should write these tables.

create table world_meta (
    id smallint primary key check (id = 1),
    tick bigint not null,
    next_id bigint not null,
    saved_at timestamptz not null
);

create table systems (
    key text primary key,
    state jsonb not null
);

create table characters (
    account text primary key,
    state jsonb not null,
    updated_at timestamptz not null
);
