-- The test prompts a training draws at each checkpoint (JSON list of strings), made
-- from the dataset's captions or written by hand. Older trainings have none.
ALTER TABLE trainings ADD COLUMN sample_prompts TEXT;
