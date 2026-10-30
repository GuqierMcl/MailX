CREATE UNIQUE INDEX draft_attachments_inline_content_id
    ON draft_attachments(draft_id, content_id)
    WHERE inline = 1;
