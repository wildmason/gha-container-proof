FROM alpine:3.20

COPY entrypoint.sh /entrypoint.sh
COPY pre.sh /pre.sh
COPY post.sh /post.sh

RUN chmod +x /entrypoint.sh /pre.sh /post.sh

ENTRYPOINT ["/entrypoint.sh"]
