pub mod channel;
pub mod post;
pub mod refresh_token;
pub mod server;
pub mod text_channel;
pub mod user;
pub mod user_server;
pub mod user_text_channel;
pub mod user_user;
pub mod voice_channel;

/*
 * main entities are:
 * users -> other users (friends), posts
 * servers -> users (members), voice channels, text channels -> posts
 *
 * posts can either belong to a:
 * friends relationship, text channel
 *
 * all entity relationships for the following set of features are ready
 *
 * Users:
 * Platform can have users. X
 * Users have a username X and profile picture []
 * Users have a status: online, away, dnd, offline. X
 * Users can have and manage friends. Users can also message friends directly. X
 * Users can join, leave and create servers. X
 * Users can also join temporary voice calls with friends directly, and invite more friends to
 * calls. X
 *
 * Servers:
 * servers can have text and voice channels, managed by admins of that server X
 * admins are creators and any users promoted by the creator X
 * creators can delete servers, but no one else can X
 * voice channels can be joined and left by users X
 * text channels can have message posts from users in that server X
 * servers can notify users when messages are sent by others X
 *
 * */
