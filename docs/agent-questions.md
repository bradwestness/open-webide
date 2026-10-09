# Agent questions

In tool-enabled chat, the agent can use `ask_user_question` to clarify a choice
before continuing. It works in local and remote projects and project-less chat,
including SSH host administration.

Questions appear above the conversation. A request can contain up to three
related questions, with optional choices and a free-text answer. Select a choice
or enter your own answer for each question, then choose **Submit answers**.
Recommended choices are suggestions; nothing is selected or submitted for you.
Keyboard users can move between choices with the arrow keys, enter text, and
activate the submit button with Enter or Space. Ctrl+Enter (Cmd+Enter on macOS)
also submits complete answers.

The agent waits for the reply and receives the submitted answers as its tool
result. Questions and answers live in SQLite, so another browser or device can
answer the same pending question. Answers are visible to the agent and retained
with the conversation. Use private terminal replies for passwords and terminal
prompts, and the approval controls for permission to run a tool.

**Cancel questions** tells the agent that you declined to answer; it may continue
without that information. **Stop** ends the run and cancels its pending questions.
Replies to completed questions or questions from an earlier run are rejected.
Switching project, session or account clears the previous form.

Bridge runs keep waiting if the browser disconnects. A local project runs in its
browser, so closing that browser interrupts execution. Its saved question can
still be answered; reopen the project folder and choose **Resume** to continue
from the saved answer. Answer or cancel pending questions before resuming.
Completed tool calls are not replayed. Changing browser requires granting access to the local folder again.

The tool is included with the normal tool set. Model tool selection and the
session's tools setting still apply. It does not need an SSH connection or
permission to execute commands.
