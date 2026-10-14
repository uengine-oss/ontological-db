# AeonG (PVLDB 2024, https://github.com/hououou/AeonG) 를 소스에서 빌드한다.
#
# 공식 이미지 hououou/aeong:v1 은 바이너리 없는 CentOS 7 / amd64 빌드 환경이라
# Apple Silicon 에서는 QEMU 아래 yum·curl·sbcl 이 vsyscall 문제로 죽는다.
# 그래서 Ubuntu 22.04 에서 네이티브(arm64 또는 amd64)로 빌드한다.
#
#   docker build -t aeong -f bench/aeong/Dockerfile bench/aeong
#   docker run -d --name aeong -p 17687:7687 aeong                      # 논리 타임스탬프
#   docker run -d --name aeong -p 17687:7687 aeong --real_time_flag=true  # epoch ms
#   echo "MATCH (n) TT AS 5 RETURN n;" | docker exec -i aeong mgconsole --host 127.0.0.1 --output-format csv
FROM ubuntu:22.04
# vldb 브랜치(기본 브랜치), 2024-05-24 — docs/aeong-temporal-comparison.md 에서 시험한 커밋
ARG AEONG_REF=b780a0c
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update -qq && apt-get install -y -qq \
      git make pkg-config curl wget uuid-dev default-jre-headless libreadline-dev \
      libpython3-dev python3-dev libssl-dev libseccomp-dev netcat python3 python3-pip python3-yaml \
      libcurl4-openssl-dev sbcl autoconf libtool clang-13 lld-13 clang-format-13 cmake \
      zlib1g-dev libbz2-dev liblz4-dev libzstd-dev libsnappy-dev libfmt-dev libjemalloc-dev bzip2 \
 && ln -sf /usr/bin/clang-13 /usr/local/bin/clang \
 && ln -sf /usr/bin/clang++-13 /usr/local/bin/clang++ \
 && ln -sf /usr/bin/clang-format-13 /usr/local/bin/clang-format \
 && git config --global user.email build@local && git config --global user.name build

# Memgraph 2.3 은 Boost >= 1.78 을 요구한다 (22.04 는 1.74).
RUN cd /tmp && wget -q https://archives.boost.io/release/1.78.0/source/boost_1_78_0.tar.bz2 \
 && tar xjf boost_1_78_0.tar.bz2 && cd boost_1_78_0 && ./bootstrap.sh --prefix=/usr/local >/dev/null \
 && ./b2 -j"$(nproc)" --with-system --with-filesystem --with-thread --with-program_options \
      --with-context --with-coroutine --with-atomic --with-date_time --with-regex install >/dev/null \
 && rm -rf /tmp/boost_1_78_0*

# DEFINE_HIDDEN_* 매크로가 있는 Memgraph 의 gflags 포크 (toolchain v4 와 같은 커밋).
RUN git clone -q https://github.com/memgraph/gflags.git /tmp/gflags && cd /tmp/gflags \
 && git checkout -q b37ceb03a0e56c9f15ce80409438a555f8a67b7c && mkdir b && cd b \
 && cmake .. -DCMAKE_INSTALL_PREFIX=/usr/local -DREGISTER_INSTALL_PREFIX=OFF \
      -DBUILD_gflags_nothreads_LIB=OFF -DGFLAGS_NO_FILENAMES=0 -DCMAKE_POSITION_INDEPENDENT_CODE=ON >/dev/null \
 && make -j"$(nproc)" install >/dev/null && rm -rf /tmp/gflags

RUN git clone -q -b vldb https://github.com/hououou/AeonG.git /aeong && cd /aeong && git checkout -q "$AEONG_REF"

# LCP(C++ 코드 생성기)용 quicklisp.
RUN cd /aeong && echo '(load "/aeong/quicklisp.lisp") (quicklisp-quickstart:install :path "/root/quicklisp/")' | sbcl --script >/dev/null \
 && ln -Tfs /aeong/src/lisp /root/quicklisp/local-projects/lcp \
 && echo '(load "/root/quicklisp/setup.lisp") (ql:quickload (quote (:lcp)) :silent t)' | sbcl --script

# AeonG 저장소에는 libs/*.patch 세 개가 빠져 있다. 라이브러리 핀이 같은 Memgraph v2.3.1 에서 가져온다.
RUN cd /aeong/libs && for p in antlr4.patch rocksdb.patch librdtsc.patch; do \
      wget -q "https://raw.githubusercontent.com/memgraph/memgraph/v2.3.1/libs/$p"; done \
 && ./setup.sh >/dev/null

RUN ARCH=$([ "$(uname -m)" = aarch64 ] && echo ARM64 || echo x86_64) \
 && mkdir -p /aeong/build && cd /aeong/build \
 && cmake .. -DMG_ARCH="$ARCH" -DCMAKE_BUILD_TYPE=Release -DBOOST_ROOT=/usr/local >/dev/null \
 && make -j"$(nproc)" memgraph mgconsole \
 && ln -s /aeong/build/bin/mgconsole /usr/local/bin/mgconsole

EXPOSE 7687
WORKDIR /aeong/build
ENTRYPOINT ["./memgraph", "--bolt-address=0.0.0.0", "--bolt-port=7687", "--data-directory=/data", \
            "--telemetry-enabled=false", "--log-level=WARNING"]
