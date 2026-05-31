pub mod post;
pub mod user;

/*
 * main entities are:
 * users -> other users (friends), posts
 * servers -> users (members), voice channels, text channels -> posts
 *
 * posts can either belong to a:
 * friends relationship, text channel
 *
 * Users:
 * Platform can have users.
 * Users have a username and profile picture
 * Users have a status: online, away, dnd, offline.
 * Users can have and manage friends. Users can also message friends directly.
 * Users can join, leave and create servers.
 * Users can also join temporary voice calls with friends directly, and invite more friends to
 * calls.
 *
 * Servers:
 * servers can have text and voice channels, managed by admins of that server
 * admins are creators and any users promoted by the creator
 * creators can delete servers, but no one else can
 * voice channels can be joined and left by users
 * text channels can have message posts from users in that server
 * servers can notify users when messages are sent by others
 *
 *
 * */
