MATCH (p:Proposal {id: $id}) RETURN p.originalPrompt AS prompt, p.clarificationLog AS clog, p.intentFeedbackLog AS flog, p.strategicDiff AS sd, p.tacticalDiff AS td
