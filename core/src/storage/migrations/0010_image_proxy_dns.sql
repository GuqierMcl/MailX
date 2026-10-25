ALTER TABLE network_routes ADD COLUMN image_proxy_dns INTEGER NOT NULL DEFAULT 0
    CHECK (image_proxy_dns IN (0, 1) AND (kind = 'custom' OR image_proxy_dns = 0));
