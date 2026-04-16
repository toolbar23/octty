 ok. now another thing.

  some shells have another layer of identity. agents like codex have their own session id that you can connect to, even
  when the original shell has long been terminated.

  we want to retrieve these session ids when we start a new session.

  and when starting a new session we want to make it possible to - optionally- restart an old session.

  these need special helper code.

  add a field inner_session_id to our panes.

  shell-configs can specify a session-handler. none | codex are the two intiial options. this decides, how

  a) session ids are retrieved for new sessions
  b) how we can get a list of possible sessions to resume ( inner_session_id, time, description )



  for codex:

  a) start codex with an initial message "You are running inside Octty pane "<pane-id>"". We continuosly scan the
  ~/.codex directory that contains a json-log of all session-contens. when we find a session with the the text above,
  we can crosslink and set the inner_session_id to what we see.

  b) there should be a codex command to get that list.



  when restoring a workspace and we find, that the retach session for a shell with an inside_session_id is not there
  anymore, just resume the inside_session_id.

  the new shell button sfor a shell-type that can work with inner sessions, in the lower left corner should be two-part
  buttons. left/big part with the name just starts a new session. right/small part with "..." opens a modal that loads
  the resumable sessions and lists them. the user can click on one to resume or click a cancel button. keyboard based
  selection up/down/return/esc should work there as well.
