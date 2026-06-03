
# psql -h $PGHOST -d postgres -c "CREATE ROLE postgres WITH SUPERUSER LOGIN;"
# psql -h $PGHOST -d postgres -c "CREATE DATABASE local OWNER postgres;"

if ! pg_ctl status -D $PGDATA > /dev/null 2>&1; then
  pg_ctl start -D $PGDATA -l $PGDATA/postgres.log -o "-k $PGHOST"
fi
