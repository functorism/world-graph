CREATE TABLE triple (
    a TEXT NOT NULL,
    b TEXT NOT NULL,
    c TEXT NOT NULL,
    UNIQUE (a, b, c)
);

CREATE INDEX triple_a_b_idx ON triple (a, b);