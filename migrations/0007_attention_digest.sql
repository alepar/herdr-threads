CREATE INDEX service_notification_recipients_seat ON service_notification_recipients(seat_id);
CREATE INDEX warning_jobs_affected_seat ON warning_jobs(affected_seat_id);
CREATE INDEX prepared_unavailable_warnings_affected_seat ON prepared_unavailable_warnings(affected_seat_id);
CREATE INDEX prepared_unavailable_warnings_warning ON prepared_unavailable_warnings(warning_id);
CREATE INDEX membership_intervals_seat_thread ON membership_intervals(seat_id, thread_id);
CREATE INDEX messages_thread_warning ON messages(thread_id, sequence) WHERE kind='warn';
CREATE INDEX send_manifests_thread_warning ON send_manifests(thread_id, base_sequence) WHERE warning_count>0;
