FROM golang:1.26 AS build
WORKDIR /src
COPY go.mod *.go ./
RUN CGO_ENABLED=0 go build -trimpath -o /fake-external .

FROM gcr.io/distroless/static:nonroot
COPY --from=build /fake-external /fake-external
EXPOSE 8080
ENTRYPOINT ["/fake-external"]
